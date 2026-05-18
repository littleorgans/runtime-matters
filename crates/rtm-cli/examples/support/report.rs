use anyhow::{Result, bail};
use lilo_rm_core::{Lifecycle, RuntimeResponse};

pub fn print_spawned(response: RuntimeResponse) -> Result<Lifecycle> {
    let RuntimeResponse::Spawned {
        lifecycle,
        event,
        log_dir,
        stdout_path,
        stderr_path,
    } = response
    else {
        bail!("unexpected spawn response: {response:?}");
    };

    println!(
        "spawn OK; lifecycle state={}; runtime event={}; runtime_pid={} log_dir={} stdout_path={} stderr_path={}",
        lifecycle.state,
        rtm_cli::cli::event_name(&event),
        lifecycle
            .runtime_pid
            .expect("running lifecycle runtime pid"),
        rtm_cli::cli::display_optional_path(log_dir.as_deref()),
        rtm_cli::cli::display_optional_path(stdout_path.as_deref()),
        rtm_cli::cli::display_optional_path(stderr_path.as_deref())
    );

    Ok(lifecycle)
}
