#[cfg(target_os = "macos")]
pub use crate::kqueue::{ProcessExitWatcher, watch_process_exit};

#[cfg(not(target_os = "macos"))]
use anyhow::Result;
#[cfg(not(target_os = "macos"))]
use tokio::sync::oneshot;

#[cfg(not(target_os = "macos"))]
pub struct ProcessExitWatcher;

#[cfg(not(target_os = "macos"))]
pub fn watch_process_exit(_pid: u32) -> Result<(ProcessExitWatcher, oneshot::Receiver<()>)> {
    anyhow::bail!("process exit watching is not available on this platform")
}
