use rtm_platform::process_exit::{ProcessExitWatcher, watch_process_exit};
use tokio::sync::oneshot;

#[test]
fn process_exit_watcher_api_is_platform_neutral() {
    let _watcher_fn: fn(u32) -> anyhow::Result<(ProcessExitWatcher, oneshot::Receiver<()>)> =
        watch_process_exit;
}
