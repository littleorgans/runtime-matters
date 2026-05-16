use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use rtm_core::{RuntimeKind, RuntimeResponse, RuntimeRpc, SpawnRequest};
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
}

impl Cli {
    pub async fn run(self) -> Result<()> {
        match self.command {
            Command::Daemon { command } => command.run().await,
            Command::Spawn(args) => spawn(args).await,
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
                lifecycle.runtime_pid
            );
        }
        other => anyhow::bail!("unexpected spawn response: {other:?}"),
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
            for lifecycle in lifecycles {
                println!(
                    "session_id={} state={} runtime={} runtime_pid={} start_time={}",
                    lifecycle.session_id,
                    lifecycle.state,
                    lifecycle.runtime,
                    lifecycle.runtime_pid,
                    lifecycle.start_time.to_rfc3339()
                );
            }
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
        }
    }
    Ok(())
}

pub fn event_name(event: &rtm_core::RuntimeEvent) -> &'static str {
    match event {
        rtm_core::RuntimeEvent::Running { .. } => "Running",
    }
}
