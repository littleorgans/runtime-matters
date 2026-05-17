use std::os::unix::process::ExitStatusExt;
use std::process::{Command, ExitStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::Args;
use rtm_core::{LaunchSpec, RuntimeExit, RuntimeSignal, ShimExit, ShimLaunchRequest, ShimReady};
use uuid::Uuid;

pub const SHIM_RECONNECT_MAX_ATTEMPTS: usize = 10;
const SHIM_RECONNECT_INITIAL_DELAY: Duration = Duration::from_secs(1);
const SHIM_RECONNECT_MAX_DELAY: Duration = Duration::from_secs(30);
const RUNTIME_WAIT_POLL: Duration = Duration::from_millis(100);

static SIGTERM_RECEIVED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Args)]
pub struct ShimArgs {
    #[arg(long)]
    session_id: Uuid,
}

pub async fn run(args: ShimArgs) -> Result<()> {
    tokio::task::spawn_blocking(move || run_for_session_blocking(args.session_id))
        .await
        .context("shim task join failed")?
}

pub fn run_for_session_blocking(session_id: Uuid) -> Result<()> {
    let socket_path = rtm_daemon::socket::socket_path_from_env()?;
    let launch_request = ShimLaunchRequest { session_id };
    let launch = reconnecting("ShimLaunch", || {
        rtm_daemon::shim_socket::request_launch_blocking(&socket_path, launch_request.clone())
    })?;
    let mut child = runtime_command(&launch)?
        .spawn()
        .context("failed to spawn runtime")?;
    let runtime_pid = child.id();
    let tmux_pane = match rtm_platform::tmux::TmuxGateway::discover_blocking(session_id) {
        Ok(tmux_pane) => tmux_pane,
        Err(error) => {
            tracing::warn!(%error, "failed to discover tmux pane");
            None
        }
    };

    let ready = ShimReady {
        session_id,
        shim_pid: std::process::id(),
        runtime_pid,
        start_time: rtm_platform::process::start_time_for_pid(runtime_pid)?
            .unwrap_or_else(chrono::Utc::now),
        tmux_pane,
    };
    reconnecting("ShimReady", || {
        rtm_daemon::shim_socket::send_ready_blocking(&socket_path, ready.clone())
    })?;

    let status = wait_for_runtime(&mut child)?;
    let exit = ShimExit {
        session_id,
        exit: runtime_exit(status),
    };
    reconnecting("ShimExit", || {
        rtm_daemon::shim_socket::send_exit_blocking(&socket_path, exit.clone())
    })?;
    Ok(())
}

fn reconnecting<T, F>(label: &'static str, mut operation: F) -> Result<T>
where
    F: FnMut() -> Result<T>,
{
    let mut delay = SHIM_RECONNECT_INITIAL_DELAY;
    for attempt in 1..=SHIM_RECONNECT_MAX_ATTEMPTS {
        match operation() {
            Ok(value) => return Ok(value),
            Err(error) if attempt == SHIM_RECONNECT_MAX_ATTEMPTS => {
                bail!("{label} failed after {SHIM_RECONNECT_MAX_ATTEMPTS} attempts: {error}");
            }
            Err(error) => {
                tracing::warn!(%error, attempt, label, "shim reconnect attempt failed");
                thread::sleep(delay);
                delay = std::cmp::min(delay * 2, SHIM_RECONNECT_MAX_DELAY);
            }
        }
    }
    unreachable!("reconnect loop returns on success or final failure")
}

fn runtime_command(launch: &LaunchSpec) -> Result<Command> {
    let mut command = Command::new(launch.command()?);
    command.args(launch.argv.iter().skip(1));
    for env in &launch.env {
        command.env(&env.key, &env.value);
    }
    if let Some(cwd) = &launch.cwd {
        command.current_dir(cwd);
    }
    Ok(command)
}

fn wait_for_runtime(child: &mut std::process::Child) -> Result<ExitStatus> {
    install_sigterm_handler()?;
    loop {
        if let Some(status) = child.try_wait().context("failed to poll runtime child")? {
            return Ok(status);
        }
        if SIGTERM_RECEIVED.swap(false, Ordering::SeqCst) {
            rtm_platform::signal::send_signal(child.id(), RuntimeSignal::Term)?;
            return child
                .wait()
                .context("failed to wait for runtime child after SIGTERM");
        }
        thread::sleep(RUNTIME_WAIT_POLL);
    }
}

fn install_sigterm_handler() -> Result<()> {
    // SAFETY: the handler only flips an atomic flag, which is async-signal-safe.
    let previous = unsafe {
        libc::signal(
            libc::SIGTERM,
            mark_sigterm as *const () as libc::sighandler_t,
        )
    };
    if previous == libc::SIG_ERR {
        return Err(std::io::Error::last_os_error()).context("failed to install SIGTERM handler");
    }
    Ok(())
}

extern "C" fn mark_sigterm(_: libc::c_int) {
    SIGTERM_RECEIVED.store(true, Ordering::SeqCst);
}

fn runtime_exit(status: ExitStatus) -> RuntimeExit {
    RuntimeExit::new(status.code(), status.signal())
}
