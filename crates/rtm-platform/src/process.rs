#[cfg(any(target_os = "macos", target_os = "linux"))]
use anyhow::Context;
use anyhow::Result;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use chrono::TimeZone;
use chrono::{DateTime, Utc};
#[cfg(target_os = "linux")]
use std::path::Path;

#[cfg(any(target_os = "macos", target_os = "linux"))]
const START_TIME_READ_ATTEMPTS: usize = 5;
#[cfg(any(target_os = "macos", target_os = "linux"))]
const START_TIME_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessStartTime {
    Known(DateTime<Utc>),
    Gone,
    Unsupported,
}

pub fn pid_alive(pid: u32) -> bool {
    // SAFETY: signal 0 asks the kernel to validate pid without delivering a
    // signal.
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if result == 0 {
        return true;
    }

    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn start_time_probe_for_pid(pid: u32) -> Result<ProcessStartTime> {
    for attempt in 1..=START_TIME_READ_ATTEMPTS {
        let probe = read_start_time_probe_for_pid(pid)?;
        if probe != ProcessStartTime::Gone || !pid_alive(pid) || attempt == START_TIME_READ_ATTEMPTS
        {
            return Ok(probe);
        }
        std::thread::sleep(START_TIME_RETRY_DELAY);
    }

    Ok(ProcessStartTime::Gone)
}

#[cfg(target_os = "macos")]
fn read_start_time_probe_for_pid(pid: u32) -> Result<ProcessStartTime> {
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

    if read != size as libc::c_int {
        let error = std::io::Error::last_os_error();
        if (read == 0 && !pid_alive(pid)) || error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(ProcessStartTime::Gone);
        }
        return Err(error).with_context(|| format!("failed to read start time for pid {pid}"));
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
    Ok(ProcessStartTime::Known(timestamp))
}

#[cfg(target_os = "linux")]
fn read_start_time_probe_for_pid(pid: u32) -> Result<ProcessStartTime> {
    let stat_path = format!("/proc/{pid}/stat");
    let stat = match std::fs::read_to_string(&stat_path) {
        Ok(stat) => stat,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ProcessStartTime::Gone);
        }
        Err(error) => return Err(error).with_context(|| format!("failed to read {stat_path}")),
    };

    let Some(start_ticks) = proc_stat_start_ticks(&stat) else {
        anyhow::bail!("failed to parse start time from {stat_path}");
    };

    let Some(boot_time) = linux_boot_time(Path::new("/proc/stat"))? else {
        return Ok(ProcessStartTime::Unsupported);
    };
    let Some(start_time) = start_time_from_ticks(boot_time, start_ticks)? else {
        return Ok(ProcessStartTime::Unsupported);
    };

    Ok(ProcessStartTime::Known(start_time))
}

#[cfg(target_os = "linux")]
fn proc_stat_start_ticks(stat: &str) -> Option<u64> {
    let close = stat.rfind(')')?;
    let mut fields = stat.get(close + 2..)?.split_whitespace();
    fields.nth(19)?.parse().ok()
}

#[cfg(target_os = "linux")]
fn linux_boot_time(path: &Path) -> Result<Option<i64>> {
    let stat = match std::fs::read_to_string(path) {
        Ok(stat) => stat,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()));
        }
    };

    Ok(stat.lines().find_map(|line| {
        line.strip_prefix("btime ")
            .and_then(|value| value.trim().parse().ok())
    }))
}

#[cfg(target_os = "linux")]
fn start_time_from_ticks(boot_time: i64, start_ticks: u64) -> Result<Option<DateTime<Utc>>> {
    let ticks_per_second = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if ticks_per_second <= 0 {
        return Ok(None);
    }

    let ticks_per_second = ticks_per_second as u64;
    let seconds = boot_time + (start_ticks / ticks_per_second) as i64;
    let nanos = ((start_ticks % ticks_per_second) * 1_000_000_000 / ticks_per_second) as u32;
    Utc.timestamp_opt(seconds, nanos)
        .single()
        .map(Some)
        .with_context(|| "invalid Linux process start time")
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn start_time_probe_for_pid(_pid: u32) -> Result<ProcessStartTime> {
    Ok(ProcessStartTime::Unsupported)
}

pub fn start_time_for_pid(_pid: u32) -> Result<Option<DateTime<Utc>>> {
    match start_time_probe_for_pid(_pid)? {
        ProcessStartTime::Known(start_time) => Ok(Some(start_time)),
        ProcessStartTime::Gone | ProcessStartTime::Unsupported => Ok(None),
    }
}

#[cfg(test)]
#[cfg(target_os = "macos")]
mod tests {
    use std::process::Command;
    use std::thread;
    use std::time::{Duration, Instant};

    use super::{ProcessStartTime, start_time_for_pid, start_time_probe_for_pid};

    #[test]
    fn start_time_for_pid_returns_none_for_zombie_process() {
        let mut child = Command::new("/bin/sh")
            .arg("-c")
            .arg("exit 0")
            .spawn()
            .expect("spawn short lived process");
        let pid = child.id();
        let deadline = Instant::now() + Duration::from_secs(2);

        while Instant::now() < deadline {
            match start_time_probe_for_pid(pid) {
                Ok(ProcessStartTime::Gone) => {
                    assert_eq!(start_time_for_pid(pid).expect("legacy start time"), None);
                    child.wait().expect("reap child");
                    return;
                }
                Ok(ProcessStartTime::Known(_)) => thread::sleep(Duration::from_millis(10)),
                Ok(ProcessStartTime::Unsupported) => {
                    let _ = child.wait();
                    panic!("macOS start time probe returned unsupported");
                }
                Err(error) => {
                    let _ = child.wait();
                    panic!("start time lookup failed for zombie pid {pid}: {error:#}");
                }
            }
        }

        let _ = child.wait();
        panic!("process {pid} did not reach zombie start time state");
    }
}

#[cfg(test)]
#[cfg(target_os = "linux")]
mod linux_tests {
    use std::process::Command;

    use super::{ProcessStartTime, proc_stat_start_ticks, start_time_probe_for_pid};

    #[test]
    fn proc_stat_start_ticks_handles_process_names_with_spaces() {
        let stat = "123 (name with spaces) S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 4242 20";

        assert_eq!(proc_stat_start_ticks(stat), Some(4242));
    }

    #[test]
    fn start_time_probe_reads_linux_proc_start_time() {
        let mut child = Command::new("/bin/sh")
            .arg("-c")
            .arg("sleep 1")
            .spawn()
            .expect("spawn process");
        let pid = child.id();

        match start_time_probe_for_pid(pid).expect("start time probe") {
            ProcessStartTime::Known(_) => {}
            ProcessStartTime::Gone => panic!("running child was reported gone"),
            ProcessStartTime::Unsupported => panic!("Linux /proc start time was unsupported"),
        }

        child.kill().expect("kill child");
        child.wait().expect("reap child");
        assert_eq!(
            start_time_probe_for_pid(pid).expect("start time after reap"),
            ProcessStartTime::Gone
        );
    }
}
