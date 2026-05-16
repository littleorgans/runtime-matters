use anyhow::{Context, Result};
use rtm_core::RuntimeSignal;

pub fn send_signal(pid: u32, signal: RuntimeSignal) -> Result<()> {
    let number = signal_number(signal);
    // SAFETY: kill is called with a process id supplied by rtmd state and a
    // fixed signal number from RuntimeSignal.
    let result = unsafe { libc::kill(pid as libc::pid_t, number) };
    if result == 0 {
        return Ok(());
    }
    Err(std::io::Error::last_os_error())
        .with_context(|| format!("failed to send SIG{signal} to pid {pid}"))
}

pub const fn signal_number(signal: RuntimeSignal) -> i32 {
    match signal {
        RuntimeSignal::Hup => libc::SIGHUP,
        RuntimeSignal::Int => libc::SIGINT,
        RuntimeSignal::Term => libc::SIGTERM,
        RuntimeSignal::Kill => libc::SIGKILL,
    }
}
