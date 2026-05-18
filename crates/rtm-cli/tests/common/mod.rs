#![allow(dead_code)]

pub mod mcp;
pub mod tmux;

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};
use std::{fmt::Write as _, io::Read, io::Write};

use chrono::{DateTime, TimeZone, Utc};
use rtm_core::{Lifecycle, RuntimeKind, ShimReady};
use rtm_store::{LifecycleStore, StoreConfig};
use tempfile::TempDir;
use uuid::Uuid;

pub const FAKE_RUNTIME_READY: &str = "rtm fake runtime ready";

pub struct RtmHarness {
    temp: TempDir,
    socket: PathBuf,
    db: PathBuf,
    rtm_home: PathBuf,
    rtm: PathBuf,
    daemon: Child,
    reconcile_env: Vec<(&'static str, String)>,
    start_outside_tmux: bool,
}

impl RtmHarness {
    pub fn start() -> Self {
        Self::start_with_options(Vec::new(), false)
    }

    pub fn start_outside_tmux() -> Self {
        Self::start_with_options(Vec::new(), true)
    }

    pub fn start_with_fast_resume_probe() -> Self {
        Self::start_with_options(
            vec![
                ("RTM_PROBE_SWEEP_INTERVAL_MS", "30000".to_owned()),
                ("RTM_RESUME_POLL_INTERVAL_MS", "25".to_owned()),
                ("RTM_RESUME_GAP_THRESHOLD_MS", "1".to_owned()),
            ],
            false,
        )
    }

    fn start_with_options(
        reconcile_env: Vec<(&'static str, String)>,
        start_outside_tmux: bool,
    ) -> Self {
        let temp = TempDir::new().expect("temp dir");
        let socket = temp.path().join("rtm.sock");
        let db = temp.path().join("rtm.sqlite");
        let rtm_home = temp.path().join("rtm-home");
        write_fake_runtime(temp.path(), "claude");
        write_fake_runtime(temp.path(), "codex");
        let rtm = default_rtm_path();
        let mut daemon = start_daemon(
            &rtm,
            &socket,
            &db,
            &rtm_home,
            temp.path(),
            &reconcile_env,
            start_outside_tmux,
        );
        wait_for_socket(&socket, &mut daemon);
        Self {
            temp,
            socket,
            db,
            rtm_home,
            rtm,
            daemon,
            reconcile_env,
            start_outside_tmux,
        }
    }

    pub fn spawn(&self, session_id: &str) -> Output {
        self.spawn_runtime(session_id, "claude")
    }

    pub fn spawn_runtime(&self, session_id: &str, runtime: &str) -> Output {
        self.spawn_command(session_id, runtime, "headless")
            .output()
            .expect("spawn client")
    }

    pub fn spawn_runtime_in_tmux(
        &self,
        session_id: &str,
        runtime: &str,
        tmux_address: &str,
    ) -> Output {
        self.spawn_command(session_id, runtime, &format!("tmux:{tmux_address}"))
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
        self.daemon = start_daemon(
            &self.rtm,
            &self.socket,
            &self.db,
            &self.rtm_home,
            self.temp.path(),
            &self.reconcile_env,
            self.start_outside_tmux,
        );
        wait_for_socket(&self.socket, &mut self.daemon);
    }

    pub fn stop_rtmd(&mut self) {
        stop_daemon(&self.rtm, &self.socket, &mut self.daemon);
    }

    pub fn db_path(&self) -> &Path {
        &self.db
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket
    }

    pub fn rtm_home(&self) -> &Path {
        &self.rtm_home
    }

    pub fn rtm_path(&self) -> &Path {
        &self.rtm
    }

    pub fn daemon_pid(&self) -> u32 {
        self.daemon.id()
    }

    pub fn stop(mut self) {
        self.cleanup_processes();
        stop_daemon(&self.rtm, &self.socket, &mut self.daemon);
    }

    fn rtm_command(&self) -> Command {
        let mut command = Command::new(&self.rtm);
        command.env("RTM_SOCKET_PATH", &self.socket);
        command.env("RTM_DB_PATH", &self.db);
        command
    }

    fn spawn_command(&self, session_id: &str, runtime: &str, target: &str) -> Command {
        let mut command = self.rtm_command();
        command
            .arg("spawn")
            .arg("--runtime")
            .arg(runtime)
            .arg("--session-id")
            .arg(session_id)
            .arg("--target")
            .arg(target);
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
        let _ = Command::new(&self.rtm)
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

pub fn spawn_ok(harness: &RtmHarness, session_id: &str, runtime: &str) -> String {
    let output = harness.spawn_runtime(session_id, runtime);
    spawn_output_ok(output, runtime)
}

pub fn spawn_output_ok(output: Output, runtime: &str) -> String {
    assert!(
        output.status.success(),
        "{runtime} spawn failed: {output:?}"
    );
    output_stdout(output)
}

pub fn status_pid(harness: &RtmHarness, session_id: &str, format: &str) -> u32 {
    let output = harness.status_format(session_id, format);
    assert!(output.status.success(), "status failed: {output:?}");
    parse_status_pid(&output_stdout(output))
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
    let mut last_status = String::new();
    wait_until(timeout, || {
        let output = harness.status(session_id);
        let success = output.status.success();
        let stdout = String::from_utf8(output.stdout).expect("stdout");
        let stderr = String::from_utf8(output.stderr).expect("stderr");
        last_status = format!("success={success} stdout={stdout:?} stderr={stderr:?}");
        stdout.contains(needle).then_some(stdout)
    })
    .unwrap_or_else(|| panic!("status never contained {needle}; last status: {last_status}"))
}

pub fn wait_for_events(harness: &RtmHarness, expected: usize) -> String {
    wait_until(Duration::from_secs(5), || {
        let output = harness.events();
        let stdout = output_stdout(output);
        (stdout.lines().count() == expected).then_some(stdout)
    })
    .unwrap_or_else(|| panic!("events never reached {expected}"))
}

pub fn wait_until<T>(timeout: Duration, mut check: impl FnMut() -> Option<T>) -> Option<T> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(value) = check() {
            return Some(value);
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    None
}

pub fn wait_until_not_alive(pid: u32) {
    wait_until(Duration::from_secs(5), || {
        (!process_alive(pid)).then_some(())
    })
    .unwrap_or_else(|| panic!("pid {pid} was still alive after SIGKILL"));
}

pub fn process_alive(pid: u32) -> bool {
    Command::new("ps")
        .arg("-p")
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("ps")
        .success()
}

pub fn persist_running(db_path: &Path, session_id: Uuid, runtime_pid: u32) {
    persist_running_with_start_time(
        db_path,
        session_id,
        runtime_pid,
        Utc.timestamp_opt(1_000, 0).unwrap(),
    )
}

pub fn persist_running_with_start_time(
    db_path: &Path,
    session_id: Uuid,
    runtime_pid: u32,
    start_time: DateTime<Utc>,
) {
    tokio::runtime::Runtime::new()
        .expect("tokio runtime")
        .block_on(async {
            let store = LifecycleStore::open(StoreConfig {
                db_path: db_path.to_path_buf(),
            })
            .await
            .expect("store");
            let mut lifecycle = Lifecycle::forking(session_id, RuntimeKind::Claude);
            store.insert_forking(&lifecycle).await.expect("insert");
            lifecycle.mark_running(ShimReady {
                session_id,
                shim_pid: runtime_pid + 1,
                runtime_pid,
                start_time,
                tmux_pane: None,
            });
            store.update_lifecycle(&lifecycle).await.expect("running");
        });
}

pub fn unused_pid() -> u32 {
    (60_000..61_000)
        .find(|pid| !process_alive(*pid))
        .expect("unused pid")
}

fn write_fake_runtime(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\nif [ \"${{RTM_TEST_STDIO_SENTINELS:-}}\" = 1 ]; then\n  printf 'HELLO\\n'\n  printf 'WORLD\\n' >&2\n  exit 0\nfi\nprintf '{}\\n'\nexec sleep 60\n",
            FAKE_RUNTIME_READY
        ),
    )
    .expect("fake runtime");
    let mut permissions = std::fs::metadata(&path).expect("metadata").permissions();
    use std::os::unix::fs::PermissionsExt;
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).expect("permissions");
    path
}

fn start_daemon(
    rtm: &Path,
    socket: &Path,
    db: &Path,
    rtm_home: &Path,
    fake_bin_dir: &Path,
    reconcile_env: &[(&'static str, String)],
    start_outside_tmux: bool,
) -> Child {
    let mut command = Command::new(rtm);
    command
        .arg("daemon")
        .arg("start")
        .env("RTM_SOCKET_PATH", socket)
        .env("RTM_DB_PATH", db)
        .env("RTM_HOME", rtm_home)
        .env("PATH", test_path(fake_bin_dir))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if start_outside_tmux {
        command.env_remove("TMUX").env_remove("TMUX_PANE");
    }
    for (key, value) in reconcile_env {
        command.env(key, value);
    }
    command.spawn().expect("daemon start")
}

fn test_path(fake_bin_dir: &Path) -> String {
    let current = std::env::var_os("PATH").unwrap_or_default();
    let paths = std::iter::once(fake_bin_dir.to_path_buf()).chain(std::env::split_paths(&current));
    std::env::join_paths(paths)
        .expect("joined path")
        .to_string_lossy()
        .into_owned()
}

fn wait_for_socket(socket: &Path, daemon: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last_error = None;
    while Instant::now() < deadline {
        match UnixStream::connect(socket) {
            Ok(_) => return,
            Err(error) => last_error = Some(error),
        }
        if daemon.try_wait().expect("daemon try_wait").is_some() {
            panic!(
                "daemon exited before socket accepted connections at {}{}",
                socket.display(),
                daemon_debug(daemon)
            );
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!(
        "daemon socket never accepted connections at {}; last error={last_error:?}",
        socket.display()
    );
}

fn stop_daemon(rtm: &Path, socket: &Path, daemon: &mut Child) {
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

fn default_rtm_path() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_rtm") {
        return PathBuf::from(path);
    }

    let current = std::env::current_exe().expect("current exe");
    let dir = current.parent().expect("executable parent");
    let candidate_dir = match dir.file_name().and_then(|name| name.to_str()) {
        Some("deps" | "examples") => dir.parent().expect("target profile dir"),
        _ => dir,
    };
    candidate_dir.join(format!("rtm{}", std::env::consts::EXE_SUFFIX))
}
