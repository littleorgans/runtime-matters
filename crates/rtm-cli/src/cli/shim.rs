use anyhow::{Context, Result};
use chrono::Utc;
use clap::Args;
use rtm_core::{RuntimeKind, ShimReady};
use uuid::Uuid;

#[derive(Debug, Args)]
pub struct ShimArgs {
    #[arg(long)]
    session_id: Uuid,
    #[arg(long)]
    runtime: RuntimeKind,
}

pub async fn run(args: ShimArgs) -> Result<()> {
    let mut child = rtm_daemon::shim_socket::runtime_command(args.runtime)
        .env("RTM_SESSION_ID", args.session_id.to_string())
        .env("RTM_RUNTIME_KIND", args.runtime.as_str())
        .spawn()
        .with_context(|| format!("failed to spawn {} runtime", args.runtime))?;

    let ready = ShimReady {
        session_id: args.session_id,
        runtime_pid: child.id(),
        start_time: Utc::now(),
    };
    let socket_path = rtm_daemon::socket::socket_path_from_env()?;
    rtm_daemon::shim_socket::send_ready(&socket_path, ready).await?;

    let _ = child.wait().context("failed to wait for runtime child")?;
    Ok(())
}
