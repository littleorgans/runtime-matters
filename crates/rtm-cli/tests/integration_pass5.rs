mod common;

use std::process::{Command, Output};
use std::time::{Duration, Instant};

use common::{RtmHarness, output_stdout};
use uuid::Uuid;

#[test]
fn pass5_spawn_inside_tmux_captures_pane_and_nudges_it() {
    if !tmux_available() {
        eprintln!("skipping tmux integration test because tmux is unavailable");
        return;
    }

    let harness = RtmHarness::start();
    let session_id = Uuid::now_v7().to_string();
    let tmux_session = format!("rtm-pass5-{}", Uuid::now_v7().simple());
    tmux(["new-session", "-d", "-s", &tmux_session]);
    let expected_pane = tmux_stdout(["list-panes", "-t", &tmux_session, "-F", "#S:#I.#P"]);
    let expected_pane = expected_pane.lines().next().expect("pane").to_owned();

    send_spawn_command(&harness, &tmux_session, &session_id);
    let status = wait_for_json_status(&harness, &session_id, &expected_pane);
    assert!(
        status.contains(&format!("\"tmux_pane\": \"{expected_pane}\"")),
        "{status}"
    );

    let content = format!("hello-from-rtm-{}", Uuid::now_v7().simple());
    let nudge = harness.nudge(&session_id, &content);
    assert!(nudge.status.success(), "nudge failed: {nudge:?}");
    wait_for_tmux_capture(&tmux_session, &content);

    tmux(["kill-session", "-t", &tmux_session]);
    harness.stop();
}

fn send_spawn_command(harness: &RtmHarness, tmux_session: &str, session_id: &str) {
    let command = format!(
        "RTM_SOCKET_PATH={} RTM_DB_PATH={} {} spawn --runtime claude --session-id {}",
        harness.socket_path().display(),
        harness.db_path().display(),
        env!("CARGO_BIN_EXE_rtm"),
        session_id
    );
    tmux(["send-keys", "-t", tmux_session, "-l", &command]);
    tmux(["send-keys", "-t", tmux_session, "Enter"]);
}

fn wait_for_json_status(harness: &RtmHarness, session_id: &str, needle: &str) -> String {
    wait_until(Duration::from_secs(5), || {
        let output = harness.status_format(session_id, "json");
        let stdout = output_stdout(output);
        stdout.contains(needle).then_some(stdout)
    })
    .unwrap_or_else(|| panic!("json status never contained {needle}"))
}

fn wait_for_tmux_capture(tmux_session: &str, needle: &str) {
    wait_until(Duration::from_secs(5), || {
        let capture = tmux_stdout(["capture-pane", "-p", "-t", tmux_session]);
        capture.contains(needle).then_some(())
    })
    .unwrap_or_else(|| panic!("tmux pane never contained {needle}"));
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

fn tmux_available() -> bool {
    Command::new("tmux")
        .arg("-V")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn tmux<const N: usize>(args: [&str; N]) {
    let output = tmux_output(args);
    assert!(output.status.success(), "tmux command failed: {output:?}");
}

fn tmux_stdout<const N: usize>(args: [&str; N]) -> String {
    let output = tmux_output(args);
    assert!(output.status.success(), "tmux command failed: {output:?}");
    output_stdout(output)
}

fn tmux_output<const N: usize>(args: [&str; N]) -> Output {
    Command::new("tmux").args(args).output().expect("tmux")
}
