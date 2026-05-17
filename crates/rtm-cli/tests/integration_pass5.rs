mod common;

use common::{RtmHarness, output_stdout};
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

    tmux_session.send_spawn_command(&harness, &session_id);
    let status = wait_for_json_status(&harness, &session_id, &expected_pane);
    assert!(
        status.contains(&format!("\"tmux_pane\": \"{expected_pane}\"")),
        "{status}"
    );

    let content = format!("hello-from-rtm-{}", Uuid::now_v7().simple());
    let nudge = harness.nudge(&session_id, &content);
    assert!(nudge.status.success(), "nudge failed: {nudge:?}");
    tmux_session.wait_for_capture(&content);

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
