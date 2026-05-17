mod common;

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use chrono::{TimeZone, Utc};
use common::{RtmHarness, output_stdout, wait_for_events, wait_for_status_timeout};
use rtm_core::{Lifecycle, RuntimeKind, ShimReady};
use rtm_store::{LifecycleStore, StoreConfig};
use uuid::Uuid;

#[test]
fn pass7_periodic_reconciliation_marks_lost_and_doctor_reports_it() {
    let harness = RtmHarness::start();
    let session_id = Uuid::now_v7();
    let runtime_pid = unused_pid();
    persist_running(harness.db_path(), session_id, runtime_pid);

    let status = wait_for_status_timeout(
        &harness,
        &session_id.to_string(),
        "state=Lost(PidNotAlive)",
        Duration::from_secs(40),
    );
    assert!(status.contains("runtime=claude"), "{status}");

    let events = wait_for_events(&harness, 1);
    assert!(events.contains("runtime event=Lost"), "{events}");
    assert!(events.contains(&session_id.to_string()), "{events}");
    assert!(events.contains("evidence=PidNotAlive"), "{events}");

    let doctor = harness.doctor();
    assert!(doctor.status.success(), "doctor failed: {doctor:?}");
    let doctor = output_stdout(doctor);
    assert!(doctor.contains("rtmd"), "{doctor}");
    assert!(doctor.contains("sqlite"), "{doctor}");
    assert!(doctor.contains("applied migrations  2 of 2"), "{doctor}");
    assert!(doctor.contains("lifecycles"), "{doctor}");
    assert!(doctor.contains("lost                1"), "{doctor}");
    assert!(doctor.contains("last probe sweep"), "{doctor}");
    assert!(!doctor.contains("last probe sweep      never"), "{doctor}");
    assert!(doctor.contains("recent lost"), "{doctor}");
    assert!(doctor.contains(&session_id.to_string()), "{doctor}");
    assert!(doctor.contains("PidNotAlive"), "{doctor}");

    harness.stop();
}

fn persist_running(db_path: &Path, session_id: Uuid, runtime_pid: u32) {
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
                start_time: Utc.timestamp_opt(1_000, 0).unwrap(),
                tmux_pane: None,
            });
            store.update_lifecycle(&lifecycle).await.expect("running");
        });
}

fn unused_pid() -> u32 {
    (60_000..61_000)
        .find(|pid| !process_alive(*pid))
        .expect("unused pid")
}

fn process_alive(pid: u32) -> bool {
    Command::new("ps")
        .arg("-p")
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("ps")
        .success()
}
