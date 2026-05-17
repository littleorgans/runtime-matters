#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};
use std::{fmt::Write as _, io::Read, io::Write};

use tempfile::TempDir;

pub struct RtmHarness {
    temp: TempDir,
    socket: PathBuf,
    db: PathBuf,
    rtm: &'static str,
    daemon: Child,
}

impl RtmHarness {
    pub fn start() -> Self {
        let temp = TempDir::new().expect("temp dir");
        let socket = temp.path().join("rtm.sock");
        let db = temp.path().join("rtm.sqlite");
        write_fake_runtime(temp.path(), "claude");
        write_fake_runtime(temp.path(), "codex");
        let rtm = env!("CARGO_BIN_EXE_rtm");
        let daemon = start_daemon(rtm, &socket, &db, temp.path());
        wait_for_socket(&socket);
        Self {
            temp,
            socket,
            db,
            rtm,
            daemon,
        }
    }

    pub fn spawn(&self, session_id: &str) -> Output {
        self.spawn_runtime(session_id, "claude")
    }

    pub fn spawn_runtime(&self, session_id: &str, runtime: &str) -> Output {
        self.rtm_command()
            .arg("spawn")
            .arg("--runtime")
            .arg(runtime)
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

    pub fn nudge(&self, session_id: &str, content: &str) -> Output {
        self.rtm_command()
            .arg("nudge")
            .arg("--session-id")
            .arg(session_id)
            .arg("--content")
            .arg(content)
            .output()
            .expect("nudge client")
    }

    pub fn events(&self) -> Output {
        self.rtm_command()
            .arg("events")
            .output()
            .expect("events client")
    }

    pub fn doctor(&self) -> Output {
        self.rtm_command()
            .arg("doctor")
            .output()
            .expect("doctor client")
    }

    pub fn mcp_line(&self, line: &str) -> Output {
        let mut child = self
            .rtm_command()
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("mcp client");
        {
            let stdin = child.stdin.as_mut().expect("mcp stdin");
            stdin.write_all(line.as_bytes()).expect("write mcp line");
            stdin.write_all(b"\n").expect("write mcp newline");
        }
        child.wait_with_output().expect("mcp output")
    }

    pub fn start_rtmd(&mut self) {
        self.daemon = start_daemon(self.rtm, &self.socket, &self.db, self.temp.path());
        wait_for_socket(&self.socket);
    }

    pub fn stop_rtmd(&mut self) {
        stop_daemon(self.rtm, &self.socket, &mut self.daemon);
    }

    pub fn db_path(&self) -> &Path {
        &self.db
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket
    }

    pub fn stop(mut self) {
        self.cleanup_processes();
        stop_daemon(self.rtm, &self.socket, &mut self.daemon);
    }

    fn rtm_command(&self) -> Command {
        let mut command = Command::new(self.rtm);
        command.env("RTM_SOCKET_PATH", &self.socket);
        command.env("RTM_DB_PATH", &self.db);
        command
    }

    fn cleanup_processes(&self) {
        let output = self.rtm_command().arg("status").output();
        let Ok(output) = output else {
            return;
        };
        let stdout = output_stdout(output);
        for line in stdout.lines() {
            for key in ["runtime_pid", "shim_pid"] {
                if let Some(pid) = parse_status_field(line, key) {
                    terminate_process(pid, "KILL");
                }
            }
        }
    }
}

impl Drop for RtmHarness {
    fn drop(&mut self) {
        self.cleanup_processes();
        let _ = Command::new(self.rtm)
            .arg("daemon")
            .arg("stop")
            .env("RTM_SOCKET_PATH", &self.socket)
            .env("RTM_DB_PATH", &self.db)
            .output();
        let _ = self.daemon.kill();
        let _ = self.daemon.wait();
    }
}

pub fn output_stdout(output: Output) -> String {
    String::from_utf8(output.stdout).expect("stdout")
}

pub fn output_stderr(output: Output) -> String {
    String::from_utf8(output.stderr).expect("stderr")
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

fn parse_status_field(line: &str, key: &str) -> Option<u32> {
    line.split(&format!("{key}="))
        .nth(1)
        .and_then(|tail| tail.split_whitespace().next())
        .and_then(|value| (value != "-").then_some(value))
        .and_then(|value| value.parse().ok())
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

fn write_fake_runtime(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, "#!/bin/sh\nexec sleep 60\n").expect("fake runtime");
    let mut permissions = std::fs::metadata(&path).expect("metadata").permissions();
    use std::os::unix::fs::PermissionsExt;
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).expect("permissions");
    path
}

fn start_daemon(rtm: &'static str, socket: &Path, db: &Path, fake_bin_dir: &Path) -> Child {
    Command::new(rtm)
        .arg("daemon")
        .arg("start")
        .env("RTM_SOCKET_PATH", socket)
        .env("RTM_DB_PATH", db)
        .env("PATH", test_path(fake_bin_dir))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("daemon start")
}

fn test_path(fake_bin_dir: &Path) -> String {
    let current = std::env::var_os("PATH").unwrap_or_default();
    let paths = std::iter::once(fake_bin_dir.to_path_buf()).chain(std::env::split_paths(&current));
    std::env::join_paths(paths)
        .expect("joined path")
        .to_string_lossy()
        .into_owned()
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
    assert!(
        output.status.success(),
        "stop failed: {output:?}{}",
        daemon_debug(daemon)
    );
    wait_for_child(daemon);
    assert!(!socket.exists(), "socket was not removed");
}

fn daemon_debug(daemon: &mut Child) -> String {
    let Ok(Some(status)) = daemon.try_wait() else {
        return String::new();
    };
    let mut debug = String::new();
    let _ = write!(debug, "; daemon exited with {status}");
    if let Some(stderr) = daemon.stderr.as_mut() {
        let mut contents = String::new();
        let _ = stderr.read_to_string(&mut contents);
        let _ = write!(debug, "; daemon stderr: {contents}");
    }
    debug
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
