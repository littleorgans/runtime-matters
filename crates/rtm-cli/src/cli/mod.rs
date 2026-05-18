use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use clap::{Args, Parser, Subcommand, ValueEnum};
use lilo_rm_core::{
    KillByPidRequest, KillRequest, Lifecycle, NudgeRequest, RuntimeKind, RuntimeResponse,
    RuntimeRpc, RuntimeSignal, SpawnRequest, SpawnTarget, StatusFilter,
};
use uuid::Uuid;

use crate::cli::daemon::DaemonCommand;
use crate::generated::cli_help;

pub mod daemon;
pub mod doctor;
pub mod initdb;
pub mod mcp;
pub mod shim;
pub mod version;

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
    Spawn(SpawnArgs),
    #[command(about = cli_help::KILL_ABOUT)]
    Kill(KillArgs),
    Nudge(NudgeArgs),
    #[command(about = cli_help::STATUS_ABOUT)]
    Status(StatusArgs),
    #[command(about = cli_help::MCP_ABOUT)]
    Mcp,
    #[command(about = cli_help::VERSION_ABOUT)]
    Version,
    #[command(about = "Print rtmd substrate health diagnostics.")]
    Doctor,
    Events,
    Initdb,
    #[command(name = "__shim", hide = true)]
    Shim(shim::ShimArgs),
}

#[derive(Debug, Args)]
pub struct SpawnArgs {
    #[arg(long)]
    runtime: RuntimeKind,
    #[arg(long)]
    session_id: Uuid,
    #[arg(long, value_name = "headless|tmux:SESSION:WINDOW.PANE")]
    target: SpawnTarget,
}

#[derive(Debug, Args)]
pub struct StatusArgs {
    #[arg(long = "session-id", value_name = "UUID")]
    session_ids: Vec<Uuid>,
    #[arg(long, value_parser = parse_updated_since)]
    updated_since: Option<DateTime<Utc>>,
    #[arg(long)]
    runtime: Option<String>,
    #[arg(long)]
    state: Option<String>,
    #[arg(long, value_enum, default_value_t = StatusFormat::Summary)]
    format: StatusFormat,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum StatusFormat {
    #[value(name = "summary")]
    Summary,
    #[value(name = "pid")]
    Pid,
    #[value(name = "shim_pid")]
    ShimPid,
    #[value(name = "json")]
    Json,
}

#[derive(Debug, Args)]
pub struct KillArgs {
    #[arg(long, conflicts_with = "pid", required_unless_present = "pid")]
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
    #[arg(long)]
    session_id: Uuid,
    #[arg(long)]
    content: String,
}

impl Cli {
    pub async fn run(self) -> Result<()> {
        match self.command {
            Command::Daemon { command } => command.run().await,
            Command::Spawn(args) => spawn(args).await,
            Command::Kill(args) => kill(args).await,
            Command::Nudge(args) => nudge(args).await,
            Command::Status(args) => status(args).await,
            Command::Mcp => mcp::run().await,
            Command::Version => version::run().await,
            Command::Doctor => doctor::run().await,
            Command::Events => events().await,
            Command::Initdb => initdb::run().await,
            Command::Shim(args) => shim::run(args).await,
        }
    }
}

async fn spawn(args: SpawnArgs) -> Result<()> {
    let socket_path = crate::shared::socket_path()?;
    let cwd = lilo_rm_core::capture_caller_cwd().context("failed to capture caller cwd")?;
    let env = lilo_rm_core::capture_caller_env();
    let response = crate::shared::request(
        &socket_path,
        RuntimeRpc::Spawn {
            request: SpawnRequest {
                session_id: args.session_id,
                runtime: args.runtime,
                env,
                cwd,
                target: args.target,
            },
        },
    )
    .await?;

    match response {
        RuntimeResponse::Spawned {
            lifecycle,
            event,
            log_dir,
            stdout_path,
            stderr_path,
        } => {
            println!(
                "spawn OK; lifecycle state={}; runtime event={}; runtime_pid={} log_dir={} stdout_path={} stderr_path={}",
                lifecycle.state,
                event_name(&event),
                lifecycle
                    .runtime_pid
                    .expect("running lifecycle runtime pid"),
                display_optional_path(log_dir.as_deref()),
                display_optional_path(stdout_path.as_deref()),
                display_optional_path(stderr_path.as_deref())
            );
        }
        other => anyhow::bail!("unexpected spawn response: {other:?}"),
    }
    Ok(())
}

async fn kill(args: KillArgs) -> Result<()> {
    if let Some(pid) = args.pid {
        return kill_pid(args, pid).await;
    }
    let session_id = args
        .session_id
        .ok_or_else(|| anyhow::anyhow!("--session-id or --pid is required"))?;
    let socket_path = crate::shared::socket_path()?;
    let response = crate::shared::request(
        &socket_path,
        RuntimeRpc::Kill {
            request: KillRequest {
                session_id,
                signal: args.signal,
                grace_secs: args.grace_secs,
            },
        },
    )
    .await?;

    match response {
        RuntimeResponse::Ack => {
            println!("kill OK; session_id={session_id}");
        }
        other => bail!("unexpected kill response: {other:?}"),
    }
    Ok(())
}

async fn kill_pid(args: KillArgs, pid: u32) -> Result<()> {
    let socket_path = crate::shared::socket_path()?;
    let response = crate::shared::request(
        &socket_path,
        RuntimeRpc::KillByPid {
            request: KillByPidRequest {
                pid,
                signal: rtm_platform::signal::signal_number(args.signal),
                grace_secs: args.grace_secs,
            },
        },
    )
    .await?;

    match response {
        RuntimeResponse::KillByPid { response } => {
            println!(
                "kill OK; pid={} signal={} killed_after_grace={}",
                response.pid, response.signal, response.killed_after_grace
            );
        }
        other => bail!("unexpected kill-by-pid response: {other:?}"),
    }
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
        RuntimeResponse::Ack => {
            println!("nudge OK; session_id={}", args.session_id);
        }
        other => bail!("unexpected nudge response: {other:?}"),
    }
    Ok(())
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
        RuntimeResponse::Status { lifecycles } if lifecycles.is_empty() => {
            println!("no lifecycles");
        }
        RuntimeResponse::Status { lifecycles } => {
            print_status(args.format, &lifecycles)?;
        }
        other => anyhow::bail!("unexpected status response: {other:?}"),
    }
    Ok(())
}

async fn events() -> Result<()> {
    let socket_path = crate::shared::socket_path()?;
    let events = crate::shared::events(&socket_path).await?;
    for event in events {
        match event {
            lilo_rm_core::RuntimeEvent::Running {
                session_id,
                runtime_pid,
                start_time,
            } => println!(
                "runtime event=Running session_id={} runtime_pid={} start_time={}",
                session_id,
                runtime_pid,
                start_time.to_rfc3339()
            ),
            lilo_rm_core::RuntimeEvent::Terminated {
                session_id,
                exit_code,
                signal,
                evidence,
            } => println!(
                "runtime event=Terminated session_id={} exit_code={} signal={} evidence={}",
                session_id,
                display_optional_i32(exit_code),
                display_optional_i32(signal),
                evidence
            ),
            lilo_rm_core::RuntimeEvent::Lost {
                session_id,
                evidence,
            } => println!(
                "runtime event=Lost session_id={} evidence={}",
                session_id, evidence
            ),
        }
    }
    Ok(())
}

pub fn event_name(event: &lilo_rm_core::RuntimeEvent) -> &'static str {
    match event {
        lilo_rm_core::RuntimeEvent::Running { .. } => "Running",
        lilo_rm_core::RuntimeEvent::Terminated { .. } => "Terminated",
        lilo_rm_core::RuntimeEvent::Lost { .. } => "Lost",
    }
}

fn print_status(format: StatusFormat, lifecycles: &[Lifecycle]) -> Result<()> {
    match format {
        StatusFormat::Summary => {
            for lifecycle in lifecycles {
                println!(
                    "session_id={} state={} runtime={} shim_pid={} runtime_pid={} start_time={} tmux_pane={}",
                    lifecycle.session_id,
                    lifecycle.state,
                    lifecycle.runtime,
                    display_optional_u32(lifecycle.shim_pid),
                    display_optional_u32(lifecycle.runtime_pid),
                    lifecycle
                        .start_time
                        .map(|time| time.to_rfc3339())
                        .unwrap_or_else(|| "-".to_owned()),
                    display_optional_tmux_pane(lifecycle.tmux_pane.as_ref())
                );
            }
        }
        StatusFormat::Pid => println!("{}", one_pid(lifecycles, |row| row.runtime_pid, "pid")?),
        StatusFormat::ShimPid => {
            println!("{}", one_pid(lifecycles, |row| row.shim_pid, "shim_pid")?)
        }
        StatusFormat::Json => println!("{}", serde_json::to_string_pretty(lifecycles)?),
    }
    Ok(())
}

fn one_pid(
    lifecycles: &[Lifecycle],
    getter: impl Fn(&Lifecycle) -> Option<u32>,
    label: &'static str,
) -> Result<u32> {
    let [lifecycle] = lifecycles else {
        bail!("--format {label} requires exactly one lifecycle");
    };
    getter(lifecycle).ok_or_else(|| anyhow::anyhow!("lifecycle missing {label}"))
}

fn display_optional_u32(value: Option<u32>) -> String {
    value
        .map(|inner| inner.to_string())
        .unwrap_or_else(|| "-".to_owned())
}

fn display_optional_i32(value: Option<i32>) -> String {
    value
        .map(|inner| inner.to_string())
        .unwrap_or_else(|| "-".to_owned())
}

fn parse_updated_since(value: &str) -> std::result::Result<DateTime<Utc>, chrono::ParseError> {
    DateTime::parse_from_rfc3339(value).map(|time| time.with_timezone(&Utc))
}

fn display_optional_tmux_pane(value: Option<&lilo_rm_core::TmuxAddress>) -> String {
    value
        .map(ToString::to_string)
        .unwrap_or_else(|| "-".to_owned())
}

pub fn display_optional_path(value: Option<&std::path::Path>) -> String {
    value
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "-".to_owned())
}
