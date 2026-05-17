use std::sync::Arc;

use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use rtm_core::{Lifecycle, LostEvidence, RuntimeEvent};

use crate::server::ServerState;

pub trait ProcessProbe {
    fn pid_alive(&self, pid: u32) -> bool;
    fn start_time_for_pid(&self, pid: u32) -> Result<Option<DateTime<Utc>>>;
}

pub struct SystemProcessProbe;

impl ProcessProbe for SystemProcessProbe {
    fn pid_alive(&self, pid: u32) -> bool {
        rtm_platform::process::pid_alive(pid)
    }

    fn start_time_for_pid(&self, pid: u32) -> Result<Option<DateTime<Utc>>> {
        rtm_platform::process::start_time_for_pid(pid)
    }
}

pub async fn reconcile_startup(
    state: Arc<ServerState>,
    probe: &impl ProcessProbe,
) -> Result<Vec<RuntimeEvent>> {
    let mut events = Vec::new();
    for lifecycle in state.store().running().await? {
        if let Some(evidence) = lost_evidence(&lifecycle, probe)? {
            if let Some(event) = state.record_lost(lifecycle.session_id, evidence).await? {
                events.push(event);
            }
            continue;
        }
        let runtime_pid = lifecycle
            .runtime_pid
            .ok_or_else(|| anyhow!("running lifecycle missing runtime pid"))?;
        state
            .start_exit_watcher(lifecycle.session_id, runtime_pid)
            .await?;
    }
    Ok(events)
}

fn lost_evidence(lifecycle: &Lifecycle, probe: &impl ProcessProbe) -> Result<Option<LostEvidence>> {
    let runtime_pid = lifecycle
        .runtime_pid
        .ok_or_else(|| anyhow!("running lifecycle missing runtime pid"))?;
    if !probe.pid_alive(runtime_pid) {
        return Ok(Some(LostEvidence::PidNotAlive));
    }

    let Some(stored_start_time) = lifecycle.start_time else {
        return Ok(None);
    };
    let Some(current_start_time) = probe.start_time_for_pid(runtime_pid)? else {
        return Ok(None);
    };
    if current_start_time != stored_start_time {
        return Ok(Some(LostEvidence::PidReuseDetected));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;

    use chrono::TimeZone;
    use rtm_core::{LifecycleState, RuntimeKind, ShimReady};
    use rtm_store::{LifecycleStore, StoreConfig};
    use uuid::Uuid;

    use super::*;
    use crate::server::DaemonConfig;

    struct FakeProbe {
        alive: HashSet<u32>,
        start_times: HashMap<u32, DateTime<Utc>>,
    }

    impl ProcessProbe for FakeProbe {
        fn pid_alive(&self, pid: u32) -> bool {
            self.alive.contains(&pid)
        }

        fn start_time_for_pid(&self, pid: u32) -> Result<Option<DateTime<Utc>>> {
            Ok(self.start_times.get(&pid).copied())
        }
    }

    #[tokio::test]
    async fn startup_reconciliation_marks_dead_and_reused_pids_lost_once() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let store = LifecycleStore::open(StoreConfig {
            db_path: temp.path().join("rtm.sqlite"),
        })
        .await
        .expect("store");
        let dead = persist_running(&store, 101, Utc.timestamp_opt(1_000, 0).unwrap()).await;
        let reused = persist_running(&store, 202, Utc.timestamp_opt(2_000, 0).unwrap()).await;
        let mut already_lost =
            persist_running(&store, 303, Utc.timestamp_opt(3_000, 0).unwrap()).await;
        already_lost.mark_lost(LostEvidence::PidNotAlive);
        store
            .update_lifecycle(&already_lost)
            .await
            .expect("persist lost");

        let state = Arc::new(ServerState::new(test_config(), store.clone()));
        let probe = FakeProbe {
            alive: HashSet::from([202]),
            start_times: HashMap::from([(202, Utc.timestamp_opt(2_001, 0).unwrap())]),
        };

        let events = reconcile_startup(Arc::clone(&state), &probe)
            .await
            .expect("reconcile");
        let replay = reconcile_startup(state, &probe).await.expect("replay");

        assert_eq!(events.len(), 2);
        assert!(replay.is_empty(), "{replay:?}");
        assert_lost(&store, dead.session_id, LostEvidence::PidNotAlive).await;
        assert_lost(&store, reused.session_id, LostEvidence::PidReuseDetected).await;
    }

    async fn assert_lost(store: &LifecycleStore, session_id: Uuid, evidence: LostEvidence) {
        let lifecycle = store
            .get(session_id)
            .await
            .expect("get")
            .expect("lifecycle");
        assert_eq!(lifecycle.state, LifecycleState::Lost(evidence));
    }

    async fn persist_running(
        store: &LifecycleStore,
        pid: u32,
        start_time: DateTime<Utc>,
    ) -> Lifecycle {
        let mut lifecycle = forking_lifecycle();
        store.insert_forking(&lifecycle).await.expect("insert");
        lifecycle.mark_running(ShimReady {
            session_id: lifecycle.session_id,
            shim_pid: pid + 10_000,
            runtime_pid: pid,
            start_time,
            tmux_pane: None,
        });
        store.update_lifecycle(&lifecycle).await.expect("running");
        lifecycle
    }

    fn forking_lifecycle() -> Lifecycle {
        let session_id = Uuid::now_v7();
        Lifecycle::forking(session_id, RuntimeKind::Claude)
    }

    fn test_config() -> DaemonConfig {
        DaemonConfig {
            socket_path: PathBuf::from("/tmp/rtm-test.sock"),
            shim_path: PathBuf::from("rtm"),
            store: StoreConfig {
                db_path: PathBuf::from("/tmp/rtm-test.sqlite"),
            },
        }
    }
}
