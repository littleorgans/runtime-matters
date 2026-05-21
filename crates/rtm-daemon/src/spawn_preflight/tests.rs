use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use chrono::Utc;
use lilo_rm_core::{
    HeadlessSpawnTarget, IsolationPolicy, IsolationProfile, Lifecycle, RuntimeKind,
    RuntimeResponse, ShimReady, SpawnRequest, SpawnTarget, TmuxSpawnTarget, WatcherCounts,
};
use rtm_store::{LifecycleStore, StoreConfig};
use uuid::Uuid;

use super::*;
use crate::docker_preflight::DockerPreflightConfig;
use crate::reconcile::ReconcileConfig;
use crate::server::{DaemonConfig, ServerState};

struct FakeDockerInspector {
    availability: Result<(), &'static str>,
    user: Result<Option<&'static str>, &'static str>,
    arm64_manifest: Result<bool, &'static str>,
    image_architecture: Result<&'static str, FakeDockerImageError>,
}

impl FakeDockerInspector {
    fn available_non_root() -> Self {
        Self {
            availability: Ok(()),
            user: Ok(Some("1000")),
            arm64_manifest: Ok(true),
            image_architecture: Ok("arm64"),
        }
    }
}

#[derive(Clone, Copy)]
enum FakeDockerImageError {
    Unavailable(&'static str),
    MetadataUnavailable(&'static str),
}

impl DockerImageInspector for FakeDockerInspector {
    async fn ensure_available(&self) -> Result<()> {
        self.availability
            .map_err(|message| RuntimeFailure::docker_unavailable(message))
    }

    async fn image_user(&self, _image: &str) -> Result<Option<String>> {
        self.user
            .map(|user| user.map(ToOwned::to_owned))
            .map_err(RuntimeFailure::docker_image_unavailable)
    }

    async fn arm64_manifest_available(&self, _image: &str) -> Result<bool> {
        self.arm64_manifest
            .map_err(RuntimeFailure::docker_image_metadata_unavailable)
    }

    async fn image_architecture(&self, _image: &str) -> Result<String> {
        self.image_architecture
            .map(ToOwned::to_owned)
            .map_err(|error| match error {
                FakeDockerImageError::Unavailable(message) => {
                    RuntimeFailure::docker_image_unavailable(message)
                }
                FakeDockerImageError::MetadataUnavailable(message) => {
                    RuntimeFailure::docker_image_metadata_unavailable(message)
                }
            })
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

    let error = check_with_docker_inspector(
        &state,
        &request,
        &FakeDockerInspector {
            availability: Err("daemon socket refused"),
            user: Ok(Some("1000")),
            arm64_manifest: Ok(true),
            image_architecture: Ok("arm64"),
        },
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
async fn docker_tmux_pattern_a_passes_preflight() {
    let state = test_state().await;
    let mut request = tmux_request(Uuid::now_v7(), false);
    request.isolation = docker_profile(None);

    let response =
        check_with_docker_inspector(&state, &request, &FakeDockerInspector::available_non_root())
            .await
            .expect("docker tmux target should pass preflight");

    assert!(response.is_none(), "tmux Pattern A returned conflict");
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

        let response = check_with_docker_inspector(
            &state,
            &request,
            &FakeDockerInspector::available_non_root(),
        )
        .await
        .expect("preflight");

        assert!(response.is_none(), "accepted profile returned conflict");
    }
}

#[tokio::test]
async fn root_image_user_fails_before_lifecycle_insert() {
    assert_docker_image_user_rejected(Some("root")).await;
    assert_docker_image_user_rejected(Some("0:0")).await;
}

#[tokio::test]
async fn missing_image_user_metadata_is_treated_as_root() {
    assert_docker_image_user_rejected(None).await;
}

#[tokio::test]
async fn root_image_user_is_allowed_by_profile_escape_hatch() {
    let state = test_state().await;
    let mut request = headless_request(Uuid::now_v7(), false);
    request.isolation = docker_profile(Some("allow-root"));

    let response = check_with_docker_inspector(
        &state,
        &request,
        &FakeDockerInspector {
            availability: Ok(()),
            user: Ok(Some("root")),
            arm64_manifest: Ok(true),
            image_architecture: Ok("arm64"),
        },
    )
    .await
    .expect("preflight");

    assert!(response.is_none(), "allow-root profile returned conflict");
}

#[tokio::test]
async fn root_image_user_is_allowed_by_config_escape_hatch() {
    let state =
        test_state_with_docker_config(DockerPreflightConfig::new("test-agent:latest", true, false))
            .await;
    let mut request = headless_request(Uuid::now_v7(), false);
    request.isolation = docker_profile(None);

    let response = check_with_docker_inspector(
        &state,
        &request,
        &FakeDockerInspector {
            availability: Ok(()),
            user: Ok(Some("0")),
            arm64_manifest: Ok(true),
            image_architecture: Ok("arm64"),
        },
    )
    .await
    .expect("preflight");

    assert!(response.is_none(), "config root escape returned conflict");
}

#[tokio::test]
async fn image_unavailable_fails_before_lifecycle_insert() {
    let state = test_state().await;
    let session_id = Uuid::now_v7();
    let mut request = headless_request(session_id, false);
    request.isolation = docker_profile(None);

    let error = check_with_docker_inspector(
        &state,
        &request,
        &FakeDockerInspector {
            availability: Ok(()),
            user: Err("pull access denied"),
            arm64_manifest: Ok(true),
            image_architecture: Ok("arm64"),
        },
    )
    .await
    .expect_err("image unavailable should fail preflight");

    assert_eq!(
        error.to_string(),
        "docker image is unavailable: pull access denied"
    );
    assert_no_lifecycle_or_waiters(&state, session_id).await;
}

#[tokio::test]
async fn arm64_manifest_absence_fails_before_lifecycle_insert() {
    assert_arm64_manifest_failure(
        Ok(false),
        "docker image metadata is unavailable: docker image test-agent:latest does not publish an arm64 manifest",
    )
    .await;
}

#[tokio::test]
async fn registry_failure_with_invalid_local_metadata_preserves_metadata_category() {
    assert_arm64_manifest_failure_with_architecture(
        Err("registry authentication required"),
        Err(FakeDockerImageError::MetadataUnavailable(
            "invalid architecture metadata",
        )),
        RuntimeFailure::DockerImageMetadataUnavailable {
            message: "registry authentication required".to_owned(),
        },
    )
    .await;
}

#[tokio::test]
async fn local_only_arm64_image_passes_on_arm64_host() {
    assert_arm64_manifest_success(Err("registry authentication required"), Ok("arm64")).await;
}

#[tokio::test]
async fn local_only_non_arm64_image_fails_on_arm64_host() {
    assert_arm64_manifest_failure_with_architecture(
        Err("registry authentication required"),
        Ok("amd64"),
        RuntimeFailure::DockerImageMetadataUnavailable {
            message: "docker image test-agent:latest does not publish an arm64 manifest".to_owned(),
        },
    )
    .await;
}

#[tokio::test]
async fn nonexistent_local_image_reports_image_unavailable_after_registry_failure() {
    assert_arm64_manifest_failure_with_architecture(
        Err("registry authentication required"),
        Err(FakeDockerImageError::Unavailable(
            "No such image: test-agent:latest",
        )),
        RuntimeFailure::DockerImageUnavailable {
            message: "No such image: test-agent:latest".to_owned(),
        },
    )
    .await;
}

#[tokio::test]
async fn arm64_manifest_escape_hatch_skips_manifest_inspection() {
    let state = test_state().await;
    let mut request = headless_request(Uuid::now_v7(), false);
    request.isolation = docker_profile(Some("arm64-manifest-escape"));

    validate_docker_image_metadata_on_arch(
        &state,
        &request,
        docker_profile_ref(&request),
        &FakeDockerInspector {
            availability: Ok(()),
            user: Ok(Some("1000")),
            arm64_manifest: Err("registry authentication required"),
            image_architecture: Err(FakeDockerImageError::Unavailable(
                "No such image: test-agent:latest",
            )),
        },
        "aarch64",
    )
    .await
    .expect("preflight");
}

async fn test_state() -> Arc<ServerState> {
    test_state_with_docker_config(DockerPreflightConfig::new(
        "runtime-matters-agent:latest",
        false,
        false,
    ))
    .await
}

async fn test_state_with_docker_config(
    docker_preflight: DockerPreflightConfig,
) -> Arc<ServerState> {
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
                docker_preflight,
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
        image: None,
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
        check_with_docker_inspector(&state, &request, &FakeDockerInspector::available_non_root())
            .await
            .expect_err("docker profile should fail preflight");

    assert_eq!(error.to_string(), message);
    assert_no_lifecycle_or_waiters(&state, session_id).await;
}

async fn assert_docker_image_user_rejected(user: Option<&'static str>) {
    let state = test_state().await;
    let session_id = Uuid::now_v7();
    let mut request = headless_request(session_id, false);
    request.isolation = docker_profile(None);

    let error = check_with_docker_inspector(
        &state,
        &request,
        &FakeDockerInspector {
            availability: Ok(()),
            user: Ok(user),
            arm64_manifest: Ok(true),
            image_architecture: Ok("arm64"),
        },
    )
    .await
    .expect_err("root image user should fail preflight");

    assert_eq!(
        error.to_string(),
        "docker image metadata is unavailable: docker image runtime-matters-agent:latest runs as root"
    );
    assert_no_lifecycle_or_waiters(&state, session_id).await;
}

async fn assert_arm64_manifest_success(
    arm64_manifest: Result<bool, &'static str>,
    image_architecture: Result<&'static str, FakeDockerImageError>,
) {
    let state = test_state_with_docker_config(DockerPreflightConfig::new(
        "test-agent:latest",
        false,
        false,
    ))
    .await;
    let mut request = headless_request(Uuid::now_v7(), false);
    request.isolation = docker_profile(None);

    validate_docker_image_metadata_on_arch(
        &state,
        &request,
        docker_profile_ref(&request),
        &FakeDockerInspector {
            availability: Ok(()),
            user: Ok(Some("1000")),
            arm64_manifest,
            image_architecture,
        },
        "aarch64",
    )
    .await
    .expect("arm64 image should pass preflight");
}

async fn assert_arm64_manifest_failure_with_architecture(
    arm64_manifest: Result<bool, &'static str>,
    image_architecture: Result<&'static str, FakeDockerImageError>,
    expected: RuntimeFailure,
) {
    let state = test_state_with_docker_config(DockerPreflightConfig::new(
        "test-agent:latest",
        false,
        false,
    ))
    .await;
    let session_id = Uuid::now_v7();
    let mut request = headless_request(session_id, false);
    request.isolation = docker_profile(None);

    let error = validate_docker_image_metadata_on_arch(
        &state,
        &request,
        docker_profile_ref(&request),
        &FakeDockerInspector {
            availability: Ok(()),
            user: Ok(Some("1000")),
            arm64_manifest,
            image_architecture,
        },
        "aarch64",
    )
    .await
    .expect_err("arm64 image should fail preflight");

    assert_runtime_failure(error, expected);
    assert_no_lifecycle_or_waiters(&state, session_id).await;
}

async fn assert_arm64_manifest_failure(arm64_manifest: Result<bool, &'static str>, expected: &str) {
    let state = test_state_with_docker_config(DockerPreflightConfig::new(
        "test-agent:latest",
        false,
        false,
    ))
    .await;
    let session_id = Uuid::now_v7();
    let mut request = headless_request(session_id, false);
    request.isolation = docker_profile(None);

    let error = validate_docker_image_metadata_on_arch(
        &state,
        &request,
        docker_profile_ref(&request),
        &FakeDockerInspector {
            availability: Ok(()),
            user: Ok(Some("1000")),
            arm64_manifest,
            image_architecture: Ok("arm64"),
        },
        "aarch64",
    )
    .await
    .expect_err("arm64 manifest should fail preflight");

    assert_eq!(error.to_string(), expected);
    assert_no_lifecycle_or_waiters(&state, session_id).await;
}

fn assert_runtime_failure(error: anyhow::Error, expected: RuntimeFailure) {
    let failure = error
        .downcast_ref::<RuntimeFailure>()
        .expect("runtime failure category");
    match (failure, expected) {
        (
            RuntimeFailure::DockerImageUnavailable { message },
            RuntimeFailure::DockerImageUnavailable { message: expected },
        )
        | (
            RuntimeFailure::DockerImageMetadataUnavailable { message },
            RuntimeFailure::DockerImageMetadataUnavailable { message: expected },
        ) => assert_eq!(message, &expected),
        (actual, expected) => {
            panic!("unexpected failure category: {actual:?}, expected {expected:?}")
        }
    }
}

fn docker_profile_ref(request: &SpawnRequest) -> &IsolationProfile {
    let IsolationPolicy::Docker(profile) = &request.isolation else {
        panic!("request is not docker isolated");
    };
    profile
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
