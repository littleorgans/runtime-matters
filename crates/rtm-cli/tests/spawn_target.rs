mod common;

use common::{RtmHarness, output_stderr, output_stdout, spawn_ok};
use serde_json::Value;
use uuid::Uuid;

#[test]
fn default_headless_spawn_records_no_tmux_pane_and_rejects_nudge() {
    let harness = RtmHarness::start();
    let session_id = Uuid::now_v7().to_string();
    spawn_ok(&harness, &session_id, "claude");

    let status = harness.status_format(&session_id, "json");
    assert!(status.status.success(), "status failed: {status:?}");
    let lifecycles: Value = serde_json::from_str(&output_stdout(status)).expect("status json");
    assert_eq!(lifecycles[0]["tmux_pane"], Value::Null);

    let nudge = harness.nudge(&session_id, "headless");
    assert!(!nudge.status.success(), "nudge succeeded: {nudge:?}");
    let stderr = output_stderr(nudge);
    assert!(stderr.contains("nudge is not supported"), "{stderr}");
}
