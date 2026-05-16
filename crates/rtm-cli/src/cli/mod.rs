use anyhow::{Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use rtm_core::{
    KillRequest, Lifecycle, RuntimeKind, RuntimeResponse, RuntimeRpc, RuntimeSignal, SpawnRequest,
};
use uuid::Uuid;

use crate::cli::daemon::DaemonCommand;

pub mod daemon;
pub mod shim;

#[derive(Debug, Parser)]
#[command(name = "rtm")]
#[command(about = "runtime-matters host runtime control")]
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
    Kill(KillArgs),
    Status(StatusArgs),
    Events,
    #[command(name = "__shim", hide = true)]
    Shim(shim::ShimArgs),
}

#[derive(Debug, Args)]
pub struct SpawnArgs {
    #[arg(long)]
    runtime: RuntimeKind,
    #[arg(long)]
    session_id: Uuid,
}

#[derive(Debug, Args)]
pub struct StatusArgs {
    #[arg(long)]
    session_id: Option<Uuid>,
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
    #[arg(long)]
    session_id: Uuid,
    #[arg(long, default_value_t = RuntimeSignal::Term)]
    signal: RuntimeSignal,
    #[arg(long, default_value_t = 2)]
    grace_secs: u64,
}

impl Cli {
    pub async fn run(self) -> Result<()> {
        match self.command {
            Command::Daemon { command } => command.run().await,
            Command::Spawn(args) => spawn(args).await,
            Command::Kill(args) => kill(args).await,
            Command::Status(args) => status(args).await,
            Command::Events => events().await,
            Command::Shim(args) => shim::run(args).await,
        }
    }
}

async fn spawn(args: SpawnArgs) -> Result<()> {
    let socket_path = crate::shared::socket_path()?;
    let response = crate::shared::request(
        &socket_path,
        RuntimeRpc::Spawn {
            request: SpawnRequest {
                session_id: args.session_id,
                runtime: args.runtime,
            },
        },
    )
    .await?;

    match response {
        RuntimeResponse::Spawned { lifecycle, event } => {
            println!(
                "spawn OK; lifecycle state={}; runtime event={}; runtime_pid={}",
                lifecycle.state,
                event_name(&event),
                lifecycle
                    .runtime_pid
                    .expect("running lifecycle runtime pid")
            );
        }
        other => anyhow::bail!("unexpected spawn response: {other:?}"),
    }
    Ok(())
}

async fn kill(args: KillArgs) -> Result<()> {
    let socket_path = crate::shared::socket_path()?;
    let response = crate::shared::request(
        &socket_path,
        RuntimeRpc::Kill {
            request: KillRequest {
                session_id: args.session_id,
                signal: args.signal,
                grace_secs: args.grace_secs,
            },
        },
    )
    .await?;

    match response {
        RuntimeResponse::Ack => {
            println!("kill OK; session_id={}", args.session_id);
        }
        other => bail!("unexpected kill response: {other:?}"),
    }
    Ok(())
}

async fn status(args: StatusArgs) -> Result<()> {
    let socket_path = crate::shared::socket_path()?;
    let response = crate::shared::status(&socket_path, args.session_id).await?;
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
            rtm_core::RuntimeEvent::Running {
                session_id,
                runtime_pid,
                start_time,
            } => println!(
                "runtime event=Running session_id={} runtime_pid={} start_time={}",
                session_id,
                runtime_pid,
                start_time.to_rfc3339()
            ),
            rtm_core::RuntimeEvent::Terminated {
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
        }
    }
    Ok(())
}

pub fn event_name(event: &rtm_core::RuntimeEvent) -> &'static str {
    match event {
        rtm_core::RuntimeEvent::Running { .. } => "Running",
        rtm_core::RuntimeEvent::Terminated { .. } => "Terminated",
    }
}

fn print_status(format: StatusFormat, lifecycles: &[Lifecycle]) -> Result<()> {
    match format {
        StatusFormat::Summary => {
            for lifecycle in lifecycles {
                println!(
                    "session_id={} state={} runtime={} shim_pid={} runtime_pid={} start_time={}",
                    lifecycle.session_id,
                    lifecycle.state,
                    lifecycle.runtime,
                    display_optional_u32(lifecycle.shim_pid),
                    display_optional_u32(lifecycle.runtime_pid),
                    lifecycle
                        .start_time
                        .map(|time| time.to_rfc3339())
                        .unwrap_or_else(|| "-".to_owned())
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
