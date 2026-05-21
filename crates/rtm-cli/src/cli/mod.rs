use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use clap::{Args, Parser, Subcommand};
use lilo_rm_client::RuntimeClient;
use lilo_rm_core::{
    CaptureRequest, EventBatch, EventsPayload, IsolationPolicy, KillByPidRequest, KillRequest,
    KilledPayload, NudgeFailureReason, NudgeOutcome, NudgeRequest, RuntimeKind, RuntimeResponse,
    RuntimeRpc, RuntimeSignal, SpawnRequest, SpawnTarget, StatusFilter, ValidateTargetOutcome,
    ValidateTargetResponse,
};
use serde::Serialize;
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::cli::daemon::DaemonCommand;
use crate::generated::cli_help;

pub mod daemon;
pub mod doctor;
pub mod initdb;
pub mod mcp;
pub mod output;
pub mod shim;
pub mod version;

const CURSOR_EXPIRED_EXIT_CODE: i32 = 2;

#[derive(Debug, Parser)]
#[command(name = "rtm")]
#[command(about = "runtime-matters host runtime control")]
#[command(display_name = "runtime-matters", version = crate::VERSION)]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Daemon {
        #[command(subcommand)]
        command: DaemonCommand,
    },
    #[command(about = "Spawn a runtime process for a session.")]
    Spawn(SpawnArgs),
    #[command(about = "Signal a runtime session by id, or a process by pid.")]
    Kill(KillArgs),
    #[command(
        about = "Deliver a nudge message to a running runtime session.",
        after_help = "Failure reasons: headless_lifecycle, session_ended, tmux_pane_dead."
    )]
    Nudge(NudgeArgs),
    #[command(about = "Capture the pane snapshot for a runtime session.")]
    Capture(CaptureArgs),
    #[command(about = "Validate a spawn target without starting a runtime.")]
    ValidateTarget(ValidateTargetArgs),
    #[command(about = cli_help::STATUS_ABOUT)]
    Status(StatusArgs),
    #[command(about = cli_help::MCP_ABOUT)]
    Mcp,
    #[command(about = cli_help::VERSION_ABOUT)]
    Version(VersionArgs),
    #[command(about = "Print rtmd substrate health diagnostics.")]
    Doctor(DoctorArgs),
    Events(EventsArgs),
    Initdb,
    #[command(name = "__shim", hide = true)]
    Shim(shim::ShimArgs),
}

#[derive(Debug, Args)]
pub struct SpawnArgs {
    #[command(flatten)]
    output: output::OutputArgs,
    #[arg(long)]
    runtime: RuntimeKind,
    #[arg(long)]
    session_id: Uuid,
    #[arg(long, value_name = "headless|tmux:SESSION:WINDOW.PANE")]
    target: SpawnTarget,
    #[arg(long, default_value_t = IsolationPolicy::Host, value_name = "host|docker[:PROFILE]")]
    isolation: IsolationPolicy,
    #[arg(long, value_name = "PATH")]
    cwd: Option<PathBuf>,
    /// Pre-empt a live runtime that already occupies the requested tmux pane.
    ///
    /// Does not override session id reuse conflicts.
    #[arg(long)]
    force: bool,
}

#[derive(Debug, Args)]
pub struct StatusArgs {
    #[command(flatten)]
    output: output::OutputArgs,
    #[arg(long = "session-id", value_name = "UUID")]
    session_ids: Vec<Uuid>,
    #[arg(long, value_parser = parse_updated_since)]
    updated_since: Option<DateTime<Utc>>,
    #[arg(long)]
    runtime: Option<String>,
    #[arg(long)]
    state: Option<String>,
}

#[derive(Debug, Args)]
pub struct KillArgs {
    #[command(flatten)]
    output: output::OutputArgs,
    #[arg(
        value_name = "SESSION_ID",
        conflicts_with = "pid",
        required_unless_present = "pid"
    )]
    session_id: Option<Uuid>,
    #[arg(long, conflicts_with = "session_id")]
    pid: Option<u32>,
    #[arg(long, default_value_t = RuntimeSignal::Term)]
    signal: RuntimeSignal,
    #[arg(long, default_value_t = 2)]
    grace_secs: u64,
}

#[derive(Debug, Args)]
pub struct NudgeArgs {
    #[command(flatten)]
    output: output::OutputArgs,
    #[arg(value_name = "SESSION_ID")]
    session_id: Uuid,
    #[arg(long)]
    content: String,
}

#[derive(Debug, Args)]
pub struct CaptureArgs {
    #[command(flatten)]
    output: output::OutputArgs,
    #[arg(value_name = "SESSION_ID")]
    session_id: Uuid,
    #[arg(long)]
    scrollback_lines: Option<u32>,
}

#[derive(Debug, Args)]
pub struct ValidateTargetArgs {
    #[command(flatten)]
    output: output::OutputArgs,
    #[arg(value_name = "TARGET")]
    target: String,
}

#[derive(Debug, Args)]
pub struct EventsArgs {
    #[command(flatten)]
    output: output::OutputArgs,
    #[arg(long)]
    since: Option<lilo_rm_core::EventCursor>,
    #[arg(long)]
    wait_ms: Option<u32>,
}

#[derive(Debug, Args)]
pub struct VersionArgs {
    #[command(flatten)]
    output: output::OutputArgs,
}

#[derive(Debug, Args)]
pub struct DoctorArgs {
    #[command(flatten)]
    output: output::OutputArgs,
}

impl Cli {
    pub async fn run(self) -> Result<()> {
        match self.command {
            Command::Daemon { command } => command.run().await,
            Command::Spawn(args) => spawn(args).await,
            Command::Kill(args) => kill(args).await,
            Command::Nudge(args) => nudge(args).await,
            Command::Capture(args) => capture(args).await,
            Command::ValidateTarget(args) => validate_target(args).await,
            Command::Status(args) => status(args).await,
            Command::Mcp => mcp::run().await,
            Command::Version(args) => version::run(args.output).await,
            Command::Doctor(args) => doctor::run(args.output).await,
            Command::Events(args) => events(args).await,
            Command::Initdb => initdb::run().await,
            Command::Shim(args) => shim::run(args).await,
        }
    }
}

async fn spawn(args: SpawnArgs) -> Result<()> {
    let SpawnArgs {
        output,
        runtime,
        session_id,
        target,
        isolation,
        cwd,
        force,
    } = args;
    let cwd = spawn_cwd(cwd)?;
    let socket_path = crate::shared::socket_path()?;
    let env = lilo_rm_core::capture_caller_env();
    let shell_resume = target
        .tmux_address()
        .map(|_| lilo_rm_core::capture_shell_resume(cwd.clone()));
    let payload = RuntimeClient::new(socket_path)
        .spawn(SpawnRequest {
            session_id,
            runtime,
            isolation,
            env,
            cwd,
            target,
            force,
            shell_resume,
        })
        .await?;

    output::emit(&output, &RuntimeResponse::Spawned(payload))?;
    Ok(())
}

fn spawn_cwd(cwd: Option<PathBuf>) -> Result<PathBuf> {
    let Some(path) = cwd else {
        return lilo_rm_core::capture_caller_cwd().context("failed to capture caller cwd");
    };
    let caller_cwd = lilo_rm_core::capture_caller_cwd().context("failed to capture caller cwd")?;
    let resolved = resolve_caller_path(&caller_cwd, &path);
    let canonical = std::fs::canonicalize(&resolved)
        .with_context(|| format!("spawn cwd does not exist: {}", resolved.display()))?;
    if !canonical.is_dir() {
        bail!("spawn cwd is not a directory: {}", canonical.display());
    }
    Ok(canonical)
}

fn resolve_caller_path(caller_cwd: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        caller_cwd.join(path)
    }
}

async fn kill(args: KillArgs) -> Result<()> {
    if let Some(pid) = args.pid {
        return kill_pid(args, pid).await;
    }
    let session_id = args
        .session_id
        .ok_or_else(|| anyhow::anyhow!("session id or --pid is required"))?;
    let client = RuntimeClient::new(crate::shared::socket_path()?);
    let outcome = client
        .kill(KillRequest {
            session_id,
            signal: args.signal,
            grace_secs: args.grace_secs,
        })
        .await?;
    output::emit(&args.output, &KilledPayload { outcome })?;
    Ok(())
}

async fn kill_pid(args: KillArgs, pid: u32) -> Result<()> {
    let client = RuntimeClient::new(crate::shared::socket_path()?);
    let outcome = client
        .kill_by_pid(KillByPidRequest {
            pid,
            signal: rtm_platform::signal::signal_number(args.signal),
            grace_secs: args.grace_secs,
        })
        .await?;
    output::emit(&args.output, &KilledPayload { outcome })?;
    Ok(())
}

async fn nudge(args: NudgeArgs) -> Result<()> {
    let socket_path = crate::shared::socket_path()?;
    let response = crate::shared::request(
        &socket_path,
        RuntimeRpc::Nudge {
            request: NudgeRequest {
                session_id: args.session_id,
                content: args.content,
            },
        },
    )
    .await?;

    match response {
        RuntimeResponse::Nudge(payload) if payload.response.delivered => {
            output::emit(&args.output, &payload.response)?
        }
        RuntimeResponse::Nudge(payload) => match payload.response.outcome {
            NudgeOutcome::Unsupported(reason) => bail!(
                "nudge unsupported; reason={} session_id={}",
                reason.as_str(),
                args.session_id
            ),
            NudgeOutcome::Failed(reason) => {
                bail!("{}", nudge_failed_message(reason, args.session_id))
            }
            NudgeOutcome::Delivered => bail!("inconsistent nudge response: {:?}", payload.response),
        },
        other => bail!("unexpected nudge response: {other:?}"),
    }
    Ok(())
}

fn nudge_failed_message(reason: NudgeFailureReason, session_id: Uuid) -> String {
    match reason {
        NudgeFailureReason::SessionEnded => format!(
            "nudge failed; reason={} session_id={} detail=session is no longer running",
            reason.as_str(),
            session_id
        ),
        _ => format!(
            "nudge failed; reason={} session_id={}",
            reason.as_str(),
            session_id
        ),
    }
}

async fn capture(args: CaptureArgs) -> Result<()> {
    let socket_path = crate::shared::socket_path()?;
    let response = crate::shared::request(
        &socket_path,
        RuntimeRpc::Capture {
            request: CaptureRequest {
                session_id: args.session_id,
                scrollback_lines: args.scrollback_lines,
            },
        },
    )
    .await?;

    match response {
        RuntimeResponse::Capture(payload) => match payload.response.into_result() {
            Ok(snapshot) => output::emit(&args.output, &snapshot)?,
            Err(error) => bail!(
                "capture failed; error={error:?} session_id={}",
                args.session_id
            ),
        },
        other => bail!("unexpected capture response: {other:?}"),
    }
    Ok(())
}

async fn validate_target(args: ValidateTargetArgs) -> Result<()> {
    let socket_path = crate::shared::socket_path()?;
    let response = RuntimeClient::new(socket_path)
        .validate_target(&args.target)
        .await?;
    emit_validate_target(&args.output, &args.target, &response)?;
    if !response.valid {
        std::process::exit(1);
    }
    Ok(())
}

fn emit_validate_target(
    args: &output::OutputArgs,
    target: &str,
    response: &ValidateTargetResponse,
) -> Result<()> {
    match args.format {
        output::OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(response)?);
        }
        output::OutputFormat::Human => {
            let validity = if response.valid { "valid" } else { "invalid" };
            println!(
                "{target}: {validity} ({})",
                validate_target_outcome_name(&response.outcome)
            );
        }
    }
    Ok(())
}

fn validate_target_outcome_name(outcome: &ValidateTargetOutcome) -> &'static str {
    match outcome {
        ValidateTargetOutcome::Valid => "Valid",
        ValidateTargetOutcome::InvalidTarget { .. } => "InvalidTarget",
        ValidateTargetOutcome::TmuxPaneDead { .. } => "TmuxPaneDead",
        ValidateTargetOutcome::UnsupportedTarget { .. } => "UnsupportedTarget",
    }
}

async fn status(args: StatusArgs) -> Result<()> {
    let socket_path = crate::shared::socket_path()?;
    let response = crate::shared::status_filtered(
        &socket_path,
        StatusFilter {
            session_id: None,
            session_ids: args.session_ids,
            updated_since: args.updated_since,
            runtime: args.runtime,
            state: args.state,
        },
    )
    .await?;
    match response {
        RuntimeResponse::Status(payload) => output::emit(&args.output, &payload.lifecycles)?,
        other => anyhow::bail!("unexpected status response: {other:?}"),
    }
    Ok(())
}

async fn events(args: EventsArgs) -> Result<()> {
    let socket_path = crate::shared::socket_path()?;
    let batch = crate::shared::events(&socket_path, args.since, args.wait_ms).await?;
    match batch {
        EventBatch::Events { events, cursor } => {
            output::emit(&args.output, &EventsPayload { events, cursor })?;
        }
        EventBatch::CursorExpired { oldest } => emit_cursor_expired(args.output, oldest)?,
        _ => bail!("unexpected events batch"),
    }
    Ok(())
}

fn emit_cursor_expired(
    args: output::OutputArgs,
    latest_cursor: lilo_rm_core::EventCursor,
) -> Result<()> {
    match args.format {
        output::OutputFormat::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&CursorExpiredOutput {
                    cursor_expired: true,
                    latest_cursor,
                })?
            );
        }
        output::OutputFormat::Human => {
            eprintln!("cursor expired (latest_cursor: {latest_cursor})");
        }
    }
    std::process::exit(CURSOR_EXPIRED_EXIT_CODE);
}

#[derive(Serialize)]
struct CursorExpiredOutput {
    cursor_expired: bool,
    latest_cursor: lilo_rm_core::EventCursor,
}

fn parse_updated_since(value: &str) -> std::result::Result<DateTime<Utc>, chrono::ParseError> {
    DateTime::parse_from_rfc3339(value).map(|time| time.with_timezone(&Utc))
}
