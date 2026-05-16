#[cfg(target_os = "macos")]
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use anyhow::Result;
use tokio::sync::oneshot;

#[cfg(target_os = "macos")]
pub struct ProcessExitWatcher {
    cancel: Arc<AtomicBool>,
}

#[cfg(not(target_os = "macos"))]
pub struct ProcessExitWatcher;

#[cfg(target_os = "macos")]
impl Drop for ProcessExitWatcher {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

#[cfg(target_os = "macos")]
pub fn watch_process_exit(pid: u32) -> Result<(ProcessExitWatcher, oneshot::Receiver<()>)> {
    let kq = unsafe { libc::kqueue() };
    if kq < 0 {
        return Err(Into::into(std::io::Error::last_os_error()));
    }

    let change = process_exit_event(pid, libc::EV_ADD | libc::EV_ENABLE | libc::EV_ONESHOT);
    let registered =
        unsafe { libc::kevent(kq, &change, 1, std::ptr::null_mut(), 0, std::ptr::null()) };
    if registered < 0 {
        let error = std::io::Error::last_os_error();
        unsafe {
            libc::close(kq);
        }
        return Err(Into::into(error));
    }

    let cancel = Arc::new(AtomicBool::new(false));
    let (sender, receiver) = oneshot::channel();
    std::thread::spawn({
        let cancel = Arc::clone(&cancel);
        move || wait_for_exit(kq, pid, cancel, sender)
    });

    Ok((ProcessExitWatcher { cancel }, receiver))
}

#[cfg(not(target_os = "macos"))]
pub fn watch_process_exit(_pid: u32) -> Result<(ProcessExitWatcher, oneshot::Receiver<()>)> {
    anyhow::bail!("kqueue process exit watching is only available on macOS")
}

#[cfg(target_os = "macos")]
fn wait_for_exit(kq: libc::c_int, pid: u32, cancel: Arc<AtomicBool>, sender: oneshot::Sender<()>) {
    let mut event = empty_event();
    let timeout = libc::timespec {
        tv_sec: 0,
        tv_nsec: 100_000_000,
    };

    loop {
        if cancel.load(Ordering::Acquire) {
            unregister_and_close(kq, pid);
            return;
        }

        let result = unsafe { libc::kevent(kq, std::ptr::null(), 0, &mut event, 1, &timeout) };
        if result > 0 {
            let _ = sender.send(());
            unsafe {
                libc::close(kq);
            }
            return;
        }
        if result < 0 {
            unsafe {
                libc::close(kq);
            }
            return;
        }
    }
}

#[cfg(target_os = "macos")]
fn unregister_and_close(kq: libc::c_int, pid: u32) {
    let change = process_exit_event(pid, libc::EV_DELETE);
    unsafe {
        libc::kevent(kq, &change, 1, std::ptr::null_mut(), 0, std::ptr::null());
        libc::close(kq);
    }
}

#[cfg(target_os = "macos")]
fn process_exit_event(pid: u32, flags: u16) -> libc::kevent {
    libc::kevent {
        ident: pid as libc::uintptr_t,
        filter: libc::EVFILT_PROC,
        flags,
        fflags: libc::NOTE_EXIT,
        data: 0,
        udata: std::ptr::null_mut(),
    }
}

#[cfg(target_os = "macos")]
fn empty_event() -> libc::kevent {
    process_exit_event(0, 0)
}
