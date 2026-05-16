use std::os::unix::process::ExitStatusExt;
use std::process::ExitStatus;

use anyhow::{Context, Result, anyhow};
use clap::Args;
use rtm_core::{RuntimeExit, RuntimeKind, RuntimeSignal, ShimExit, ShimReady};
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
    let runtime_pid = child
        .id()
        .ok_or_else(|| anyhow!("spawned {} runtime has no pid", args.runtime))?;

    let ready = ShimReady {
        session_id: args.session_id,
        shim_pid: std::process::id(),
        runtime_pid,
        start_time: rtm_platform::process::start_time_for_pid(runtime_pid)?
            .unwrap_or_else(chrono::Utc::now),
    };
    let socket_path = rtm_daemon::socket::socket_path_from_env()?;
    rtm_daemon::shim_socket::send_ready(&socket_path, ready).await?;

    let status = wait_for_runtime(&mut child).await?;
    let exit = ShimExit {
        session_id: args.session_id,
        exit: runtime_exit(status),
    };
    rtm_daemon::shim_socket::send_exit(&socket_path, exit).await?;
    Ok(())
}

async fn wait_for_runtime(child: &mut tokio::process::Child) -> Result<ExitStatus> {
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        status = child.wait() => status.context("failed to wait for runtime child"),
        _ = sigterm.recv() => {
            if let Some(pid) = child.id() {
                rtm_platform::signal::send_signal(pid, RuntimeSignal::Term)?;
            }
            child.wait().await.context("failed to wait for runtime child after SIGTERM")
        }
    }
}

fn runtime_exit(status: ExitStatus) -> RuntimeExit {
    RuntimeExit::new(status.code(), status.signal())
}
