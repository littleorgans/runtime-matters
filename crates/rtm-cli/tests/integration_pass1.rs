use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use tempfile::TempDir;
use uuid::Uuid;

#[test]
fn pass1_spawn_records_running_lifecycle_and_event() {
    let temp = TempDir::new().expect("temp dir");
    let socket = temp.path().join("rtm.sock");
    let fake_claude = write_fake_claude(temp.path());
    let rtm = env!("CARGO_BIN_EXE_rtm");
    let mut daemon = start_daemon(rtm, &socket, &fake_claude);

    wait_for_socket(&socket);
    let session_id = Uuid::now_v7().to_string();
    let spawn = Command::new(rtm)
        .arg("spawn")
        .arg("--runtime")
        .arg("claude")
        .arg("--session-id")
        .arg(&session_id)
        .env("RTM_SOCKET_PATH", &socket)
        .output()
        .expect("spawn client");

    let stdout = String::from_utf8(spawn.stdout).expect("spawn stdout");
    assert!(spawn.status.success(), "spawn failed: {stdout}");
    assert!(stdout.contains("spawn OK"));
    assert!(stdout.contains("lifecycle state=Running"));
    assert!(stdout.contains("runtime event=Running"));

    let runtime_pid = parse_runtime_pid(&stdout);
    assert_process_alive(runtime_pid);
    assert_status_contains(rtm, &socket, &session_id);
    assert_one_running_event(rtm, &socket, &session_id);
    stop_daemon(rtm, &socket, &mut daemon);
    terminate_runtime(runtime_pid);
}

fn write_fake_claude(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("claude");
    std::fs::write(&path, "#!/bin/sh\nexec sleep 60\n").expect("fake claude");
    let mut permissions = std::fs::metadata(&path).expect("metadata").permissions();
    use std::os::unix::fs::PermissionsExt;
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).expect("permissions");
    path
}

fn start_daemon(rtm: &str, socket: &Path, fake_claude: &Path) -> Child {
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

fn parse_runtime_pid(stdout: &str) -> u32 {
    stdout
        .split("runtime_pid=")
        .nth(1)
        .and_then(|tail| tail.split_whitespace().next())
        .and_then(|value| value.parse().ok())
        .expect("runtime pid in spawn output")
}

fn assert_process_alive(pid: u32) {
    let status = Command::new("ps")
        .arg("-p")
        .arg(pid.to_string())
        .status()
        .expect("ps");
    assert!(status.success(), "runtime pid {pid} is not alive");
}

fn assert_status_contains(rtm: &str, socket: &Path, session_id: &str) {
    let output = Command::new(rtm)
        .arg("status")
        .arg("--session-id")
        .arg(session_id)
        .env("RTM_SOCKET_PATH", socket)
        .output()
        .expect("status client");
    let stdout = String::from_utf8(output.stdout).expect("status stdout");
    assert!(output.status.success(), "status failed: {stdout}");
    assert!(stdout.contains("state=Running"), "{stdout}");
    assert!(stdout.contains(session_id), "{stdout}");
}

fn assert_one_running_event(rtm: &str, socket: &Path, session_id: &str) {
    let output = Command::new(rtm)
        .arg("events")
        .env("RTM_SOCKET_PATH", socket)
        .output()
        .expect("events client");
    let stdout = String::from_utf8(output.stdout).expect("events stdout");
    assert!(output.status.success(), "events failed: {stdout}");

    let lines: Vec<_> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "{stdout}");
    assert!(lines[0].contains("runtime event=Running"), "{stdout}");
    assert!(lines[0].contains(session_id), "{stdout}");
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

fn terminate_runtime(pid: u32) {
    let _ = Command::new("kill").arg(pid.to_string()).status();
}
