mod common;

use common::{FAKE_RUNTIME_READY, RtmHarness, output_stdout, spawn_ok, spawn_output_ok};
use lilo_rm_core::{CaptureError, CaptureRequest, CaptureResponse, RuntimeResponse, RuntimeRpc};
use uuid::Uuid;

#[test]
fn pass5_spawn_inside_tmux_captures_pane_and_nudges_it() {
    let Some(tmux_session) = common::tmux::TmuxSession::start("rtm-pass5") else {
        eprintln!("skipping tmux integration test because tmux is unavailable");
        return;
    };

    let harness = RtmHarness::start();
    let session_id = Uuid::now_v7().to_string();
    let expected_pane = tmux_session.pane();

    let spawn = harness.spawn_runtime_in_tmux(&session_id, "claude", &expected_pane);
    spawn_output_ok(spawn, "claude");
    let status = wait_for_json_status(&harness, &session_id, &expected_pane);
    assert!(
        status.contains(&format!("\"tmux_pane\": \"{expected_pane}\"")),
        "{status}"
    );
    tmux_session.assert_pane_listed(&expected_pane);
    tmux_session.wait_for_capture(FAKE_RUNTIME_READY);
    let capture = tmux_session.capture();
    assert!(!capture.contains("$ "), "{capture}");

    let content = format!("hello-from-rtm-{}", Uuid::now_v7().simple());
    let nudge = harness.nudge(&session_id, &content);
    assert!(nudge.status.success(), "nudge failed: {nudge:?}");
    let nudge_stdout = output_stdout(nudge);
    assert!(nudge_stdout.contains("nudge delivered"), "{nudge_stdout}");
    tmux_session.wait_for_capture(&content);

    harness.stop();
}

#[test]
fn capture_tmux_pane_returns_snapshot_json() {
    let Some(tmux_session) = common::tmux::TmuxSession::start("rtm-capture") else {
        eprintln!("skipping tmux integration test because tmux is unavailable");
        return;
    };

    let harness = RtmHarness::start();
    let session_id = Uuid::now_v7().to_string();
    let expected_pane = tmux_session.pane();
    spawn_output_ok(
        harness.spawn_runtime_in_tmux(&session_id, "claude", &expected_pane),
        "claude",
    );
    tmux_session.resize_height(5);
    tmux_session.wait_for_capture(FAKE_RUNTIME_READY);

    let marker = format!("capture-marker-{}", Uuid::now_v7().simple());
    let scrollback_payload = (0..80)
        .map(|line| format!("{marker}-{line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let nudge = harness.nudge(&session_id, &scrollback_payload);
    assert!(nudge.status.success(), "nudge failed: {nudge:?}");
    tmux_session.wait_for_capture(&marker);

    let response = capture_rpc(&harness, session_id.parse().expect("session id"), Some(200));
    let RuntimeResponse::Capture {
        response: CaptureResponse::Captured(snapshot),
    } = response
    else {
        panic!("unexpected capture response: {response:?}");
    };
    assert_eq!(snapshot.scrollback_lines_requested, 200);
    assert!(snapshot.content.contains(&marker), "{snapshot:?}");
    assert!(snapshot.scrollback_lines_included > 0, "{snapshot:?}");

    let output = harness.cli(&["capture", &session_id, "--scrollback-lines", "200"]);
    assert!(output.status.success(), "capture CLI failed: {output:?}");
    let snapshot: lilo_rm_core::PaneSnapshot =
        serde_json::from_str(&output_stdout(output)).expect("pane snapshot json");
    assert!(snapshot.content.contains(&marker), "{snapshot:?}");

    harness.stop();
}

#[test]
fn capture_headless_target_returns_not_tmux_target() {
    let harness = RtmHarness::start();
    let session_id = Uuid::now_v7();
    spawn_ok(&harness, &session_id.to_string(), "claude");

    let response = capture_rpc(&harness, session_id, None);
    let RuntimeResponse::Capture {
        response: CaptureResponse::Failed(CaptureError::NotATmuxTarget),
    } = response
    else {
        panic!("unexpected capture response: {response:?}");
    };

    harness.stop();
}

#[test]
fn capture_dead_tmux_pane_returns_pane_unavailable() {
    let Some(tmux_session) = common::tmux::TmuxSession::start("rtm-capture-dead") else {
        eprintln!("skipping tmux integration test because tmux is unavailable");
        return;
    };

    let harness = RtmHarness::start();
    let session_id = Uuid::now_v7();
    let expected_pane = tmux_session.pane();
    spawn_output_ok(
        harness.spawn_runtime_in_tmux(&session_id.to_string(), "claude", &expected_pane),
        "claude",
    );
    tmux_session.wait_for_capture(FAKE_RUNTIME_READY);
    tmux_session.kill();

    let response = capture_rpc(&harness, session_id, None);
    let RuntimeResponse::Capture {
        response: CaptureResponse::Failed(CaptureError::PaneUnavailable),
    } = response
    else {
        panic!("unexpected capture response: {response:?}");
    };

    harness.stop();
}

fn wait_for_json_status(harness: &RtmHarness, session_id: &str, needle: &str) -> String {
    common::wait_until(std::time::Duration::from_secs(5), || {
        let output = harness.status_format(session_id, "json");
        let stdout = output_stdout(output);
        stdout.contains(needle).then_some(stdout)
    })
    .unwrap_or_else(|| panic!("json status never contained {needle}"))
}

fn capture_rpc(
    harness: &RtmHarness,
    target_id: Uuid,
    scrollback_lines: Option<u32>,
) -> RuntimeResponse {
    tokio::runtime::Runtime::new()
        .expect("tokio runtime")
        .block_on(rtm_cli::shared::request(
            harness.socket_path(),
            RuntimeRpc::Capture {
                request: CaptureRequest {
                    target_id,
                    scrollback_lines,
                },
            },
        ))
        .expect("capture rpc")
}
