mod common;

use std::time::Duration;

use common::{
    RtmHarness, output_stderr, output_stdout, persist_running, spawn_ok, status_pid,
    terminate_process, unused_pid, wait_for_events, wait_for_status, wait_for_status_timeout,
    wait_until, wait_until_not_alive,
};
use uuid::Uuid;

#[test]
fn sigkill_runtime_transitions_to_exited() {
    let harness = RtmHarness::start();
    let session_id = Uuid::now_v7().to_string();
    spawn_ok(&harness, &session_id, "claude");
    let runtime_pid = status_pid(&harness, &session_id, "pid");

    terminate_process(runtime_pid, "KILL");

    let status = wait_for_status_timeout(
        &harness,
        &session_id,
        "state=Exited",
        Duration::from_secs(2),
    );
    assert!(status.contains("signal=9"), "{status}");
    let events = wait_for_events(&harness, 2);
    assert!(events.contains("runtime event=Terminated"), "{events}");
}

#[test]
fn sigkill_shim_is_reported_lost_after_runtime_exit() {
    let harness = RtmHarness::start();
    let session_id = Uuid::now_v7().to_string();
    spawn_ok(&harness, &session_id, "claude");
    let shim_pid = status_pid(&harness, &session_id, "shim_pid");
    let runtime_pid = status_pid(&harness, &session_id, "pid");

    terminate_process(shim_pid, "KILL");
    terminate_process(runtime_pid, "KILL");

    let status = wait_for_status_timeout(
        &harness,
        &session_id,
        "state=Lost(ShimDiedBeforeReport)",
        Duration::from_secs(2),
    );
    assert!(status.contains(&session_id), "{status}");
    let events = wait_for_events(&harness, 2);
    assert!(events.contains("runtime event=Lost"), "{events}");
}

#[test]
fn rtmd_restart_keeps_live_sessions_running() {
    let mut harness = RtmHarness::start();
    let session_id = Uuid::now_v7().to_string();
    spawn_ok(&harness, &session_id, "claude");

    harness.stop_rtmd();
    harness.start_rtmd();

    let status = wait_for_status(&harness, &session_id, "state=Running");
    assert!(status.contains("runtime=claude"), "{status}");
}

#[test]
fn rtmd_restart_reconciles_dead_sessions_lost() {
    let mut harness = RtmHarness::start();
    let session_id = Uuid::now_v7().to_string();
    spawn_ok(&harness, &session_id, "claude");
    let runtime_pid = status_pid(&harness, &session_id, "pid");

    harness.stop_rtmd();
    terminate_process(runtime_pid, "KILL");
    wait_until_not_alive(runtime_pid);
    harness.start_rtmd();

    let status = wait_for_status(&harness, &session_id, "state=Lost(PidNotAlive)");
    assert!(status.contains("runtime=claude"), "{status}");
}

#[test]
fn resume_gap_reconciliation_does_not_wait_for_full_sweep_interval() {
    let harness = RtmHarness::start_with_fast_resume_probe();
    let session_id = Uuid::now_v7();
    persist_running(harness.db_path(), session_id, unused_pid());

    let status = wait_for_status_timeout(
        &harness,
        &session_id.to_string(),
        "state=Lost(PidNotAlive)",
        Duration::from_secs(3),
    );
    assert!(status.contains("runtime=claude"), "{status}");
}

#[test]
fn tmux_pane_closed_while_session_alive_rejects_nudge() {
    let Some(tmux_session) = common::tmux::TmuxSession::start("rtm-critical") else {
        eprintln!("skipping tmux critical scenario because tmux is unavailable");
        return;
    };
    let harness = RtmHarness::start();
    let session_id = Uuid::now_v7().to_string();
    let expected_pane = tmux_session.pane();

    tmux_session.send_spawn_command(&harness, &session_id);
    let status = wait_for_json_status(&harness, &session_id, &expected_pane);
    assert!(status.contains(&expected_pane), "{status}");
    tmux_session.kill();

    let nudge = harness.nudge(&session_id, "closed-pane");
    assert!(
        !nudge.status.success(),
        "nudge unexpectedly succeeded: {nudge:?}"
    );
    let stderr = output_stderr(nudge);
    assert!(
        stderr.contains("not alive") || stderr.contains("failed"),
        "{stderr}"
    );
    let status = wait_for_status(&harness, &session_id, "state=Running");
    assert!(status.contains(&session_id), "{status}");
}

fn wait_for_json_status(harness: &RtmHarness, session_id: &str, needle: &str) -> String {
    wait_until(Duration::from_secs(5), || {
        let output = harness.status_format(session_id, "json");
        let stdout = output_stdout(output);
        stdout.contains(needle).then_some(stdout)
    })
    .unwrap_or_else(|| panic!("json status never contained {needle}"))
}
