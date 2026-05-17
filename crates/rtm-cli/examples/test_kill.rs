use anyhow::{Result, bail};
use clap::Parser;
use rtm_core::{KillRequest, RuntimeResponse, RuntimeRpc, RuntimeSignal};
use uuid::Uuid;

#[derive(Debug, Parser)]
struct Args {
    #[arg(long)]
    session_id: Uuid,
    #[arg(long, default_value_t = RuntimeSignal::Term)]
    signal: RuntimeSignal,
    #[arg(long, default_value_t = 2)]
    grace_secs: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let socket_path = rtm_cli::shared::socket_path()?;
    let response = rtm_cli::shared::request(
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
            Ok(())
        }
        other => bail!("unexpected kill response: {other:?}"),
    }
}
