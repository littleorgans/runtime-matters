#[path = "support/report.rs"]
mod report_support;
#[path = "support/spawn.rs"]
mod spawn_support;

use anyhow::Result;
use clap::Parser;
use rtm_core::{RuntimeKind, SpawnTarget};
use uuid::Uuid;

#[derive(Debug, Parser)]
struct Args {
    #[arg(long)]
    runtime: RuntimeKind,
    #[arg(long)]
    session_id: Uuid,
    #[arg(long, value_name = "headless|tmux:SESSION:WINDOW.PANE")]
    target: SpawnTarget,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let socket_path = rtm_cli::shared::socket_path()?;
    let response =
        spawn_support::spawn_runtime(&socket_path, args.session_id, args.runtime, args.target)
            .await?;
    let events = rtm_cli::shared::events(&socket_path).await?;

    report_support::print_spawned(response)?;
    println!("runtime events observed={}", events.len());
    Ok(())
}
