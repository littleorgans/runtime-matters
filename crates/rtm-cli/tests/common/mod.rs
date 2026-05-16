#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use tempfile::TempDir;

pub struct RtmHarness {
    _temp: TempDir,
    socket: PathBuf,
    rtm: &'static str,
    daemon: Child,
}

impl RtmHarness {
    pub fn start() -> Self {
        let temp = TempDir::new().expect("temp dir");
        let socket = temp.path().join("rtm.sock");
        let fake_claude = write_fake_claude(temp.path());
        let rtm = env!("CARGO_BIN_EXE_rtm");
        let daemon = start_daemon(rtm, &socket, &fake_claude);
        wait_for_socket(&socket);
        Self {
            _temp: temp,
            socket,
            rtm,
            daemon,
        }
    }

    pub fn spawn(&self, session_id: &str) -> Output {
        self.rtm_command()
            .arg("spawn")
            .arg("--runtime")
            .arg("claude")
            .arg("--session-id")
            .arg(session_id)
            .output()
            .expect("spawn client")
    }

    pub fn kill(&self, session_id: &str, signal: &str, grace_secs: u64) -> Output {
        self.rtm_command()
            .arg("kill")
            .arg("--session-id")
            .arg(session_id)
            .arg("--signal")
            .arg(signal)
            .arg("--grace-secs")
            .arg(grace_secs.to_string())
            .output()
            .expect("kill client")
    }

    pub fn status(&self, session_id: &str) -> Output {
        self.rtm_command()
            .arg("status")
            .arg("--session-id")
            .arg(session_id)
            .output()
            .expect("status client")
    }

    pub fn status_format(&self, session_id: &str, format: &str) -> Output {
        self.rtm_command()
            .arg("status")
            .arg("--session-id")
            .arg(session_id)
            .arg("--format")
            .arg(format)
            .output()
            .expect("status client")
    }

    pub fn events(&self) -> Output {
        self.rtm_command()
            .arg("events")
            .output()
            .expect("events client")
    }

    pub fn stop(mut self) {
        stop_daemon(self.rtm, &self.socket, &mut self.daemon);
    }

    fn rtm_command(&self) -> Command {
        let mut command = Command::new(self.rtm);
        command.env("RTM_SOCKET_PATH", &self.socket);
        command
    }
}

impl Drop for RtmHarness {
    fn drop(&mut self) {
        let _ = Command::new(self.rtm)
            .arg("daemon")
            .arg("stop")
            .env("RTM_SOCKET_PATH", &self.socket)
            .output();
        let _ = self.daemon.kill();
        let _ = self.daemon.wait();
    }
}

pub fn output_stdout(output: Output) -> String {
    String::from_utf8(output.stdout).expect("stdout")
}

pub fn parse_runtime_pid(stdout: &str) -> u32 {
    stdout
        .split("runtime_pid=")
        .nth(1)
        .and_then(|tail| tail.split_whitespace().next())
        .and_then(|value| value.parse().ok())
        .expect("runtime pid in spawn output")
}

pub fn parse_status_pid(stdout: &str) -> u32 {
    stdout.trim().parse().expect("status pid")
}

pub fn assert_process_alive(pid: u32) {
    let status = Command::new("ps")
        .arg("-p")
        .arg(pid.to_string())
        .status()
        .expect("ps");
    assert!(status.success(), "runtime pid {pid} is not alive");
}

pub fn terminate_process(pid: u32, signal: &str) {
    let _ = Command::new("kill")
        .arg(format!("-{signal}"))
        .arg(pid.to_string())
        .stderr(Stdio::null())
        .status();
}

pub fn wait_for_status(harness: &RtmHarness, session_id: &str, needle: &str) -> String {
    wait_for_status_timeout(harness, session_id, needle, Duration::from_secs(5))
}

pub fn wait_for_status_timeout(
    harness: &RtmHarness,
    session_id: &str,
    needle: &str,
    timeout: Duration,
) -> String {
    wait_until(timeout, || {
        let output = harness.status(session_id);
        let stdout = output_stdout(output);
        stdout.contains(needle).then_some(stdout)
    })
    .unwrap_or_else(|| panic!("status never contained {needle}"))
}

pub fn wait_for_events(harness: &RtmHarness, expected: usize) -> String {
    wait_until(Duration::from_secs(5), || {
        let output = harness.events();
        let stdout = output_stdout(output);
        (stdout.lines().count() == expected).then_some(stdout)
    })
    .unwrap_or_else(|| panic!("events never reached {expected}"))
}

fn wait_until<T>(timeout: Duration, mut check: impl FnMut() -> Option<T>) -> Option<T> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(value) = check() {
            return Some(value);
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    None
}

fn write_fake_claude(dir: &Path) -> PathBuf {
    let path = dir.join("claude");
    std::fs::write(&path, "#!/bin/sh\nexec sleep 60\n").expect("fake claude");
    let mut permissions = std::fs::metadata(&path).expect("metadata").permissions();
    use std::os::unix::fs::PermissionsExt;
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).expect("permissions");
    path
}

fn start_daemon(rtm: &'static str, socket: &Path, fake_claude: &Path) -> Child {
    Command::new(rtm)
        .arg("daemon")
        .arg("start")
        .env("RTM_SOCKET_PATH", socket)
        .env("RTM_CLAUDE_PATH", fake_claude)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("daemon start")
}

fn wait_for_socket(socket: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if socket.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("daemon socket never appeared at {}", socket.display());
}

fn stop_daemon(rtm: &str, socket: &Path, daemon: &mut Child) {
    let output = Command::new(rtm)
        .arg("daemon")
        .arg("stop")
        .env("RTM_SOCKET_PATH", socket)
        .output()
        .expect("daemon stop");
    assert!(output.status.success(), "stop failed: {output:?}");
    wait_for_child(daemon);
    assert!(!socket.exists(), "socket was not removed");
}

fn wait_for_child(child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if child.try_wait().expect("try wait").is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let _ = child.kill();
    panic!("daemon did not exit");
}
