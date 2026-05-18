use anyhow::{Result, bail};
use lilo_rm_core::{CliOutput, Lifecycle, RuntimeResponse};

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
    let mut rendered = String::new();
    RuntimeResponse::Spawned {
        lifecycle: lifecycle.clone(),
        event,
        log_dir,
        stdout_path,
        stderr_path,
    }
    .render_human(&mut rendered)?;
    print!("{rendered}");

    Ok(lifecycle)
}
