mod common;

use common::{RtmHarness, output_stderr, output_stdout, spawn_ok, wait_for_log};
use lilo_rm_core::{
    HeadlessSpawnTarget, LaunchEnv, RuntimeKind, RuntimeResponse, RuntimeRpc, SpawnRequest,
    SpawnTarget,
};
use serde_json::Value;
use uuid::Uuid;

#[test]
fn explicit_headless_spawn_records_no_tmux_pane_and_rejects_nudge() {
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
    assert!(
        stderr.contains(&format!(
            "nudge not supported for headless lifecycle {session_id}"
        )),
        "{stderr}"
    );
}

#[test]
fn headless_spawn_pipes_stdout_and_stderr_to_session_logs() {
    let harness = RtmHarness::start_outside_tmux();
    let session_id = Uuid::now_v7();
    let response = tokio::runtime::Runtime::new()
        .expect("tokio runtime")
        .block_on(rtm_cli::shared::request(
            harness.socket_path(),
            RuntimeRpc::Spawn {
                request: SpawnRequest {
                    session_id,
                    runtime: RuntimeKind::Claude,
                    env: vec![LaunchEnv::new("RTM_TEST_STDIO_SENTINELS", "1")],
                    cwd: harness.rtm_home().to_path_buf(),
                    target: SpawnTarget::Headless(HeadlessSpawnTarget {}),
                },
            },
        ))
        .expect("headless spawn");

    let RuntimeResponse::Spawned {
        lifecycle, log_dir, ..
    } = response
    else {
        panic!("unexpected spawn response: {response:?}");
    };
    let log_dir = log_dir.expect("headless log dir");
    assert_eq!(lifecycle.tmux_pane, None);
    assert_eq!(
        log_dir,
        harness.rtm_home().join("logs").join(session_id.to_string())
    );
    wait_for_log(log_dir.join("stdout.log"), "HELLO\n");
    wait_for_log(log_dir.join("stderr.log"), "WORLD\n");
}
