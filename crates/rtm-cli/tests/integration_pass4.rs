mod common;

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use common::{
    RtmHarness, output_stdout, parse_status_pid, terminate_process, wait_for_events,
    wait_for_status, wait_for_status_timeout,
};
use rtm_store::{LifecycleStore, StoreConfig};
use uuid::Uuid;

#[test]
fn pass4_restart_reconciles_sqlite_lifecycles() {
    let mut harness = RtmHarness::start();
    let sid1 = Uuid::now_v7().to_string();
    let sid2 = Uuid::now_v7().to_string();
    let sid3 = Uuid::now_v7().to_string();

    spawn(&harness, &sid1, "claude");
    spawn(&harness, &sid2, "claude");
    spawn(&harness, &sid3, "codex");
    let pid2 = status_pid(&harness, &sid2, "pid");

    harness.stop_rtmd();
    terminate_process(pid2, "KILL");
    wait_until_not_alive(pid2);
    harness.start_rtmd();

    let status1 = wait_for_status(&harness, &sid1, "state=Running");
    assert!(status1.contains("runtime=claude"), "{status1}");
    let status2 = wait_for_status_timeout(
        &harness,
        &sid2,
        "state=Lost(PidNotAlive)",
        Duration::from_secs(5),
    );
    assert!(status2.contains("runtime=claude"), "{status2}");
    let status3 = wait_for_status(&harness, &sid3, "state=Running");
    assert!(status3.contains("runtime=codex"), "{status3}");

    let events = wait_for_events(&harness, 1);
    assert!(events.contains("runtime event=Lost"), "{events}");
    assert!(events.contains(&sid2), "{events}");
    assert!(events.contains("evidence=PidNotAlive"), "{events}");

    let states = persisted_states(harness.db_path());
    assert_eq!(states.len(), 3, "{states:?}");
    assert_eq!(states.get(&sid1).map(String::as_str), Some("Running"));
    assert_eq!(
        states.get(&sid2).map(String::as_str),
        Some("Lost(PidNotAlive)")
    );
    assert_eq!(states.get(&sid3).map(String::as_str), Some("Running"));
}

fn spawn(harness: &RtmHarness, session_id: &str, runtime: &str) {
    let output = harness.spawn_runtime(session_id, runtime);
    assert!(
        output.status.success(),
        "{runtime} spawn failed: {output:?}"
    );
}

fn status_pid(harness: &RtmHarness, session_id: &str, format: &str) -> u32 {
    let output = harness.status_format(session_id, format);
    assert!(output.status.success(), "status failed: {output:?}");
    parse_status_pid(&output_stdout(output))
}

fn wait_until_not_alive(pid: u32) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if !process_alive(pid) {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("pid {pid} was still alive after SIGKILL");
}

fn process_alive(pid: u32) -> bool {
    std::process::Command::new("ps")
        .arg("-p")
        .arg(pid.to_string())
        .status()
        .expect("ps")
        .success()
}

fn persisted_states(db_path: &Path) -> HashMap<String, String> {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    runtime.block_on(async {
        let store = LifecycleStore::open(StoreConfig {
            db_path: db_path.to_path_buf(),
        })
        .await
        .expect("store");
        store
            .list(None)
            .await
            .expect("lifecycles")
            .into_iter()
            .map(|lifecycle| {
                (
                    lifecycle.session_id.to_string(),
                    lifecycle.state.to_string(),
                )
            })
            .collect()
    })
}
