use anyhow::{Context, Result};
use rtm_core::RuntimeSignal;

pub fn send_signal(pid: u32, signal: RuntimeSignal) -> Result<()> {
    send_raw_signal(pid, signal_number(signal))
        .with_context(|| format!("failed to send SIG{signal} to pid {pid}"))
}

pub fn send_raw_signal(pid: u32, signal: i32) -> Result<()> {
    // SAFETY: kill is called with a process id supplied by rtmd state and a
    // signal number supplied by the admin surface.
    let result = unsafe { libc::kill(pid as libc::pid_t, signal) };
    if result == 0 {
        return Ok(());
    }
    Err(std::io::Error::last_os_error())
        .with_context(|| format!("failed to send signal {signal} to pid {pid}"))
}

pub const fn signal_number(signal: RuntimeSignal) -> i32 {
    match signal {
        RuntimeSignal::Hup => libc::SIGHUP,
        RuntimeSignal::Int => libc::SIGINT,
        RuntimeSignal::Term => libc::SIGTERM,
        RuntimeSignal::Kill => libc::SIGKILL,
    }
}
