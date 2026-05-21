use std::sync::Arc;

use anyhow::Result;
use lilo_rm_core::{
    IsolationPolicy, IsolationProfile, KillRequest, RuntimeResponse, RuntimeSignal,
    SpawnConflictKind, SpawnConflictPayload, SpawnRequest, SpawnTarget,
};
use tokio::process::Command;

use crate::error::RuntimeFailure;
use crate::server::ServerState;

pub(crate) async fn check(
    state: &Arc<ServerState>,
    request: &SpawnRequest,
) -> Result<Option<RuntimeResponse>> {
    check_with_docker_availability(state, request, &DockerCliAvailability).await
}

async fn check_with_docker_availability(
    state: &Arc<ServerState>,
    request: &SpawnRequest,
    docker: &impl DockerAvailability,
) -> Result<Option<RuntimeResponse>> {
    check_isolation_policy(request, docker).await?;

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

async fn check_isolation_policy(
    request: &SpawnRequest,
    docker: &impl DockerAvailability,
) -> Result<()> {
    match &request.isolation {
        IsolationPolicy::Host => Ok(()),
        IsolationPolicy::Docker(profile) => {
            check_docker_profile(profile, &request.target, docker).await
        }
    }
}

async fn check_docker_profile(
    profile: &IsolationProfile,
    target: &SpawnTarget,
    docker: &impl DockerAvailability,
) -> Result<()> {
    if matches!(target, SpawnTarget::Tmux(_)) {
        return Err(unsupported_docker_profile(
            profile,
            "tmux target requests unsupported Pattern E",
        ));
    }

    match profile.name.as_deref() {
        None | Some("default") | Some("own-init") => docker.ensure_available().await,
        Some("pattern-e") | Some("tmux-primary") => Err(unsupported_docker_profile(
            profile,
            "requests unsupported Pattern E",
        )),
        Some("privileged") => Err(unsupported_docker_profile(
            profile,
            "requests privileged execution",
        )),
        Some(_) => Err(unsupported_docker_profile(
            profile,
            "is not an accepted Docker profile",
        )),
    }
}

fn unsupported_docker_profile(profile: &IsolationProfile, reason: &str) -> anyhow::Error {
    RuntimeFailure::unsupported_isolation_policy(format!(
        "{} ({reason})",
        IsolationPolicy::Docker(profile.clone())
    ))
}

fn conflict(kind: SpawnConflictKind, lifecycle: lilo_rm_core::Lifecycle) -> RuntimeResponse {
    RuntimeResponse::SpawnConflict(SpawnConflictPayload { kind, lifecycle })
}

trait DockerAvailability {
    async fn ensure_available(&self) -> Result<()>;
}

struct DockerCliAvailability;

impl DockerAvailability for DockerCliAvailability {
    async fn ensure_available(&self) -> Result<()> {
        let output = Command::new("docker")
            .arg("version")
            .arg("--format")
            .arg("{{.Server.Version}}")
            .output()
            .await
            .map_err(|error| RuntimeFailure::docker_unavailable(error.to_string()))?;

        if output.status.success() {
            return Ok(());
        }

        Err(RuntimeFailure::docker_unavailable(command_error_message(
            &output.stderr,
        )))
    }
}

fn command_error_message(stderr: &[u8]) -> String {
    let message = String::from_utf8_lossy(stderr).trim().to_owned();
    if message.is_empty() {
        "docker version failed without stderr".to_owned()
    } else {
        message
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use chrono::Utc;
    use lilo_rm_core::{
        HeadlessSpawnTarget, IsolationPolicy, IsolationProfile, Lifecycle, RuntimeKind,
        RuntimeResponse, ShimReady, SpawnTarget, TmuxSpawnTarget, WatcherCounts,
    };
    use rtm_store::{LifecycleStore, StoreConfig};
    use uuid::Uuid;

    use super::*;
    use crate::reconcile::ReconcileConfig;
    use crate::server::{DaemonConfig, ServerState};

    struct FakeDockerAvailability(Result<(), &'static str>);

    impl DockerAvailability for FakeDockerAvailability {
        async fn ensure_available(&self) -> Result<()> {
            self.0
                .map_err(|message| RuntimeFailure::docker_unavailable(message))
        }
    }

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

    #[tokio::test]
    async fn docker_unavailable_fails_before_lifecycle_insert() {
        let state = test_state().await;
        let session_id = Uuid::now_v7();
        let mut request = headless_request(session_id, false);
        request.isolation = docker_profile(None);

        let error = check_with_docker_availability(
            &state,
            &request,
            &FakeDockerAvailability(Err("daemon socket refused")),
        )
        .await
        .expect_err("docker unavailable should fail preflight");

        assert_eq!(
            error.to_string(),
            "docker daemon is unavailable: daemon socket refused"
        );
        assert_no_lifecycle_or_waiters(&state, session_id).await;
    }

    #[tokio::test]
    async fn docker_tmux_pattern_e_fails_before_lifecycle_insert() {
        let state = test_state().await;
        let session_id = Uuid::now_v7();
        let mut request = tmux_request(session_id, false);
        request.isolation = docker_profile(None);

        let error =
            check_with_docker_availability(&state, &request, &FakeDockerAvailability(Ok(())))
                .await
                .expect_err("docker tmux target should fail preflight");

        assert_eq!(
            error.to_string(),
            "isolation policy docker (tmux target requests unsupported Pattern E) is not supported"
        );
        assert_no_lifecycle_or_waiters(&state, session_id).await;
    }

    #[tokio::test]
    async fn docker_pattern_e_profile_fails_before_lifecycle_insert() {
        assert_docker_profile_rejected(
            "pattern-e",
            "isolation policy docker:pattern-e (requests unsupported Pattern E) is not supported",
        )
        .await;
    }

    #[tokio::test]
    async fn docker_privileged_profile_fails_before_lifecycle_insert() {
        assert_docker_profile_rejected(
            "privileged",
            "isolation policy docker:privileged (requests privileged execution) is not supported",
        )
        .await;
    }

    #[tokio::test]
    async fn unsupported_docker_profile_fails_before_lifecycle_insert() {
        assert_docker_profile_rejected(
            "locked",
            "isolation policy docker:locked (is not an accepted Docker profile) is not supported",
        )
        .await;
    }

    #[tokio::test]
    async fn accepted_docker_profiles_probe_daemon_availability() {
        for profile in [None, Some("default"), Some("own-init")] {
            let state = test_state().await;
            let mut request = headless_request(Uuid::now_v7(), false);
            request.isolation = docker_profile(profile);

            let response =
                check_with_docker_availability(&state, &request, &FakeDockerAvailability(Ok(())))
                    .await
                    .expect("preflight");

            assert!(response.is_none(), "accepted profile returned conflict");
        }
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
                    endpoint: rtm_paths::RuntimeEndpoint::unix_socket(temp.join("rtm.sock")),
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
            isolation: Default::default(),
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

    fn docker_profile(name: Option<&str>) -> IsolationPolicy {
        IsolationPolicy::Docker(IsolationProfile {
            name: name.map(ToOwned::to_owned),
        })
    }

    async fn assert_docker_profile_rejected(profile: &str, message: &str) {
        let state = test_state().await;
        let session_id = Uuid::now_v7();
        let mut request = headless_request(session_id, false);
        request.isolation = docker_profile(Some(profile));

        let error =
            check_with_docker_availability(&state, &request, &FakeDockerAvailability(Ok(())))
                .await
                .expect_err("docker profile should fail preflight");

        assert_eq!(error.to_string(), message);
        assert_no_lifecycle_or_waiters(&state, session_id).await;
    }

    async fn assert_no_lifecycle_or_waiters(state: &Arc<ServerState>, session_id: Uuid) {
        assert!(
            state
                .store()
                .get(session_id)
                .await
                .expect("store")
                .is_none(),
            "preflight inserted lifecycle row"
        );
        assert_eq!(
            state.watcher_counts().await,
            WatcherCounts {
                process_exit_watchers: 0,
                shim_sockets: 0,
                event_waiters: 0,
            }
        );
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
