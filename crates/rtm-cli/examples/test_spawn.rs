use anyhow::{Result, bail};
use clap::Parser;
use rtm_core::{RuntimeKind, RuntimeResponse, RuntimeRpc, SpawnRequest};
use uuid::Uuid;

#[derive(Debug, Parser)]
struct Args {
    #[arg(long)]
    runtime: RuntimeKind,
    #[arg(long)]
    session_id: Uuid,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let socket_path = rtm_cli::shared::socket_path()?;
    let response = rtm_cli::shared::request(
        &socket_path,
        RuntimeRpc::Spawn {
            request: SpawnRequest {
                session_id: args.session_id,
                runtime: args.runtime,
            },
        },
    )
    .await?;

    let RuntimeResponse::Spawned { lifecycle, event } = response else {
        bail!("unexpected spawn response: {response:?}");
    };
    let events = rtm_cli::shared::events(&socket_path).await?;

    println!(
        "spawn OK; lifecycle state={}; runtime event={}; runtime_pid={}",
        lifecycle.state,
        rtm_cli::cli::event_name(&event),
        lifecycle.runtime_pid
    );
    println!("runtime events observed={}", events.len());
    Ok(())
}
