use std::sync::Arc;

use anyhow::Result;
use lilo_rm_core::{
    KillRequest, RuntimeResponse, RuntimeSignal, SpawnConflictKind, SpawnConflictPayload,
    SpawnRequest,
};

use crate::server::ServerState;

pub(crate) async fn check(
    state: &Arc<ServerState>,
    request: &SpawnRequest,
) -> Result<Option<RuntimeResponse>> {
    if let Some(lifecycle) = state.store().get(request.session_id).await? {
        return Ok(Some(conflict(SpawnConflictKind::SessionId, lifecycle)));
    }

    let Some(address) = request.target.tmux_address() else {
        return Ok(None);
    };
    let Some(occupant) = state.store().running_tmux_occupant(address).await? else {
        return Ok(None);
    };
    if !request.force {
        return Ok(Some(conflict(
            SpawnConflictKind::TmuxPaneOccupancy,
            occupant,
        )));
    }

    state
        .kill_runtime(KillRequest {
            session_id: occupant.session_id,
            signal: RuntimeSignal::Term,
            grace_secs: 2,
        })
        .await?;
    Ok(None)
}

fn conflict(kind: SpawnConflictKind, lifecycle: lilo_rm_core::Lifecycle) -> RuntimeResponse {
    RuntimeResponse::SpawnConflict(SpawnConflictPayload { kind, lifecycle })
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use chrono::Utc;
    use lilo_rm_core::{
        HeadlessSpawnTarget, Lifecycle, RuntimeKind, RuntimeResponse, ShimReady, SpawnTarget,
        TmuxSpawnTarget,
    };
    use rtm_store::{LifecycleStore, StoreConfig};
    use uuid::Uuid;

    use super::*;
    use crate::reconcile::ReconcileConfig;
    use crate::server::{DaemonConfig, ServerState};

    #[tokio::test]
    async fn session_id_conflict_includes_terminal_lifecycle() {
        let state = test_state().await;
        let session_id = Uuid::now_v7();
        let mut lifecycle = Lifecycle::forking(session_id, RuntimeKind::Claude);
        state
            .store()
            .insert_forking(&lifecycle)
            .await
            .expect("insert");
        lifecycle.mark_lost(lilo_rm_core::LostEvidence::PidNotAlive);
        state
            .store()
            .update_lifecycle(&lifecycle)
            .await
            .expect("terminal");

        let response = check(&state, &headless_request(session_id, false))
            .await
            .expect("preflight")
            .expect("conflict");

        assert_conflict(response, SpawnConflictKind::SessionId, session_id);
    }

    #[tokio::test]
    async fn tmux_occupant_conflict_is_typed_without_force() {
        let state = test_state().await;
        let occupant = Uuid::now_v7();
        insert_running_tmux(&state, occupant, 60_000).await;

        let response = check(&state, &tmux_request(Uuid::now_v7(), false))
            .await
            .expect("preflight")
            .expect("conflict");

        assert_conflict(response, SpawnConflictKind::TmuxPaneOccupancy, occupant);
    }

    #[tokio::test]
    async fn force_kills_tmux_occupant_and_allows_spawn() {
        let state = test_state().await;
        let mut child = Command::new("sleep").arg("60").spawn().expect("sleep");
        let occupant = Uuid::now_v7();
        insert_running_tmux(&state, occupant, child.id()).await;

        let response = check(&state, &tmux_request(Uuid::now_v7(), true))
            .await
            .expect("preflight");

        assert!(response.is_none(), "force should clear pane conflict");
        wait_for_child_exit(&mut child);
    }

    async fn test_state() -> Arc<ServerState> {
        let temp = std::env::temp_dir().join(format!("rtm-preflight-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&temp).expect("tempdir");
        let store = LifecycleStore::open(StoreConfig {
            db_path: temp.join("rtm.sqlite"),
        })
        .await
        .expect("store");
        Arc::new(
            ServerState::new(
                DaemonConfig {
                    socket_path: temp.join("rtm.sock"),
                    shim_path: temp.join("rtm"),
                    log_root: temp.join("logs"),
                    store: StoreConfig {
                        db_path: temp.join("rtm.sqlite"),
                    },
                    reconcile: ReconcileConfig::default(),
                },
                store,
            )
            .expect("state"),
        )
    }

    async fn insert_running_tmux(state: &Arc<ServerState>, session_id: Uuid, runtime_pid: u32) {
        let mut lifecycle = Lifecycle::forking(session_id, RuntimeKind::Claude);
        state
            .store()
            .insert_forking(&lifecycle)
            .await
            .expect("insert");
        lifecycle.mark_running(ShimReady {
            session_id,
            shim_pid: runtime_pid + 1,
            runtime_pid,
            start_time: Utc::now(),
            tmux_pane: Some(tmux_address()),
        });
        state
            .store()
            .update_lifecycle(&lifecycle)
            .await
            .expect("running");
    }

    fn headless_request(session_id: Uuid, force: bool) -> SpawnRequest {
        SpawnRequest {
            session_id,
            runtime: RuntimeKind::Claude,
            env: Vec::new(),
            cwd: "/tmp".into(),
            target: SpawnTarget::Headless(HeadlessSpawnTarget {}),
            force,
            shell_resume: None,
        }
    }

    fn tmux_request(session_id: Uuid, force: bool) -> SpawnRequest {
        SpawnRequest {
            target: SpawnTarget::Tmux(TmuxSpawnTarget {
                address: tmux_address(),
            }),
            ..headless_request(session_id, force)
        }
    }

    fn tmux_address() -> lilo_rm_core::TmuxAddress {
        "rtm-test:0.1".parse().expect("tmux address")
    }

    fn assert_conflict(response: RuntimeResponse, kind: SpawnConflictKind, session_id: Uuid) {
        let RuntimeResponse::SpawnConflict(payload) = response else {
            panic!("unexpected response: {response:?}");
        };
        assert_eq!(payload.kind, kind);
        assert_eq!(payload.lifecycle.session_id, session_id);
    }

    fn wait_for_child_exit(child: &mut std::process::Child) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            match child.try_wait().expect("poll child") {
                Some(_) => return,
                None => std::thread::sleep(Duration::from_millis(25)),
            }
        }
        let _ = child.kill();
        panic!("child was still alive after force preemption");
    }
}
