use anyhow::{Context, Result};
use chrono::{DateTime, TimeZone, Utc};

pub fn pid_alive(pid: u32) -> bool {
    // SAFETY: signal 0 asks the kernel to validate pid without delivering a
    // signal.
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if result == 0 {
        return true;
    }

    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(target_os = "macos")]
pub fn start_time_for_pid(pid: u32) -> Result<Option<DateTime<Utc>>> {
    let mut info = std::mem::MaybeUninit::<libc::proc_taskallinfo>::uninit();
    let size = std::mem::size_of::<libc::proc_taskallinfo>();
    // SAFETY: proc_pidinfo writes at most `size` bytes into a valid buffer for
    // PROC_PIDTASKALLINFO. The return value tells us whether the struct is full.
    let read = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTASKALLINFO,
            0,
            info.as_mut_ptr().cast(),
            size as libc::c_int,
        )
    };

    if read == 0 && !pid_alive(pid) {
        return Ok(None);
    }
    if read != size as libc::c_int {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("failed to read start time for pid {pid}"));
    }

    // SAFETY: proc_pidinfo reported that the full proc_taskallinfo struct was
    // initialized.
    let info = unsafe { info.assume_init() };
    let timestamp = Utc
        .timestamp_opt(
            info.pbsd.pbi_start_tvsec as i64,
            (info.pbsd.pbi_start_tvusec * 1_000) as u32,
        )
        .single()
        .with_context(|| format!("invalid start time for pid {pid}"))?;
    Ok(Some(timestamp))
}

#[cfg(not(target_os = "macos"))]
pub fn start_time_for_pid(_pid: u32) -> Result<Option<DateTime<Utc>>> {
    Ok(None)
}
