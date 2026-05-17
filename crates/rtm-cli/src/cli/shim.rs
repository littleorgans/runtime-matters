use std::os::unix::process::ExitStatusExt;
use std::process::ExitStatus;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use clap::Args;
use rtm_core::{LaunchSpec, RuntimeExit, RuntimeSignal, ShimExit, ShimLaunchRequest, ShimReady};
use uuid::Uuid;

pub const SHIM_RECONNECT_MAX_ATTEMPTS: usize = 10;
const SHIM_RECONNECT_INITIAL_DELAY: Duration = Duration::from_secs(1);
const SHIM_RECONNECT_MAX_DELAY: Duration = Duration::from_secs(30);

#[derive(Debug, Args)]
pub struct ShimArgs {
    #[arg(long)]
    session_id: Uuid,
}

pub async fn run(args: ShimArgs) -> Result<()> {
    let socket_path = rtm_daemon::socket::socket_path_from_env()?;
    let launch_request = ShimLaunchRequest {
        session_id: args.session_id,
    };
    let launch = reconnecting("ShimLaunch", || {
        rtm_daemon::shim_socket::request_launch(&socket_path, launch_request.clone())
    })
    .await?;
    let mut child = runtime_command(&launch)?
        .spawn()
        .context("failed to spawn runtime")?;
    let runtime_pid = child
        .id()
        .ok_or_else(|| anyhow!("spawned runtime has no pid"))?;
    let tmux_pane = match rtm_platform::tmux::TmuxGateway::discover(args.session_id).await {
        Ok(tmux_pane) => tmux_pane,
        Err(error) => {
            tracing::warn!(%error, "failed to discover tmux pane");
            None
        }
    };

    let ready = ShimReady {
        session_id: args.session_id,
        shim_pid: std::process::id(),
        runtime_pid,
        start_time: rtm_platform::process::start_time_for_pid(runtime_pid)?
            .unwrap_or_else(chrono::Utc::now),
        tmux_pane,
    };
    reconnecting("ShimReady", || {
        rtm_daemon::shim_socket::send_ready(&socket_path, ready.clone())
    })
    .await?;

    let status = wait_for_runtime(&mut child).await?;
    let exit = ShimExit {
        session_id: args.session_id,
        exit: runtime_exit(status),
    };
    reconnecting("ShimExit", || {
        rtm_daemon::shim_socket::send_exit(&socket_path, exit.clone())
    })
    .await?;
    Ok(())
}

async fn reconnecting<T, F, Fut>(label: &'static str, mut operation: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let mut delay = SHIM_RECONNECT_INITIAL_DELAY;
    for attempt in 1..=SHIM_RECONNECT_MAX_ATTEMPTS {
        match operation().await {
            Ok(value) => return Ok(value),
            Err(error) if attempt == SHIM_RECONNECT_MAX_ATTEMPTS => {
                bail!("{label} failed after {SHIM_RECONNECT_MAX_ATTEMPTS} attempts: {error}");
            }
            Err(error) => {
                tracing::warn!(%error, attempt, label, "shim reconnect attempt failed");
                tokio::time::sleep(delay).await;
                delay = std::cmp::min(delay * 2, SHIM_RECONNECT_MAX_DELAY);
            }
        }
    }
    unreachable!("reconnect loop returns on success or final failure")
}

fn runtime_command(launch: &LaunchSpec) -> Result<tokio::process::Command> {
    let mut command = tokio::process::Command::new(launch.command()?);
    command.args(launch.argv.iter().skip(1));
    for env in &launch.env {
        command.env(&env.key, &env.value);
    }
    if let Some(cwd) = &launch.cwd {
        command.current_dir(cwd);
    }
    Ok(command)
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
