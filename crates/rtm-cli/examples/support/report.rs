use std::path::Path;

use anyhow::{Result, bail};
use rtm_core::{Lifecycle, RuntimeResponse};

pub fn print_spawned(response: RuntimeResponse) -> Result<Lifecycle> {
    let RuntimeResponse::Spawned {
        lifecycle,
        event,
        log_dir,
    } = response
    else {
        bail!("unexpected spawn response: {response:?}");
    };

    println!(
        "spawn OK; lifecycle state={}; runtime event={}; runtime_pid={} log_dir={}",
        lifecycle.state,
        rtm_cli::cli::event_name(&event),
        lifecycle
            .runtime_pid
            .expect("running lifecycle runtime pid"),
        display_optional_path(log_dir.as_deref())
    );

    Ok(lifecycle)
}

fn display_optional_path(path: Option<&Path>) -> String {
    path.map(|path| path.display().to_string())
        .unwrap_or_else(|| "-".to_owned())
}
