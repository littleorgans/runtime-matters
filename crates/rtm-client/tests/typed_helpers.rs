use std::path::PathBuf;

use lilo_rm_client::{ClientError, RuntimeClient, request};
use lilo_rm_core::{
    CaptureError, CapturePayload, CaptureRequest, CaptureResponse, CursorExpiredPayload,
    DoctorPayload, DoctorResponse, ErrorCode, EventBatch, EventsPayload, EventsRequest,
    HeadlessSpawnTarget, KillByPidPayload, KillByPidRequest, KillByPidResponse, KillRequest,
    Lifecycle, LifecycleCounts, MigrationState, NudgeFailureReason, NudgeOutcome, NudgePayload,
    NudgeRequest, NudgeResponse, RuntimeEvent, RuntimeKind, RuntimeResponse, RuntimeRpc,
    RuntimeSignal, SpawnRequest, SpawnTarget, SpawnedPayload, StatusFilter, StatusPayload,
    ValidateTargetPayload, ValidateTargetRequest, ValidateTargetResponse, VersionInfo,
    VersionPayload, WatcherCounts, read_json_line, write_json_line,
};
use tokio::io::BufReader;
use tokio::net::UnixListener;
use tokio::task::JoinHandle;
use uuid::Uuid;

fn temp_socket_path() -> (tempfile::TempDir, PathBuf) {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let socket_path = tempdir.path().join("rtmd.sock");
    (tempdir, socket_path)
}

async fn mock_response(
    expected_rpc: RuntimeRpc,
    response: RuntimeResponse,
) -> (RuntimeClient, JoinHandle<()>) {
    let (tempdir, socket_path) = temp_socket_path();
    let listener = UnixListener::bind(&socket_path).expect("bind test socket");
    let client = RuntimeClient::new(socket_path);
    let server = tokio::spawn(async move {
        let _tempdir = tempdir;
        let (stream, _) = listener.accept().await.expect("accept client");
        let (read_half, mut write_half) = stream.into_split();
        let mut reader = BufReader::new(read_half);
        let rpc: RuntimeRpc = read_json_line(&mut reader).await.expect("read rpc");
        assert_eq!(rpc, expected_rpc);
        write_json_line(&mut write_half, &response)
            .await
            .expect("write response");
    });
    (client, server)
}

#[tokio::test]
async fn missing_socket_reports_daemon_unavailable() {
    let (_tempdir, socket_path) = temp_socket_path();

    let error = request(&socket_path, RuntimeRpc::Version)
        .await
        .expect_err("missing socket should fail");

    match error {
        ClientError::DaemonUnavailable {
            socket_path: actual,
            ..
        } => assert_eq!(actual, socket_path),
        other => panic!("unexpected client error: {other:?}"),
    }
}

#[tokio::test]
async fn daemon_error_response_preserves_code() {
    let (client, server) = mock_response(
        RuntimeRpc::Version,
        RuntimeResponse::error(ErrorCode::SessionNotFound, "missing session"),
    )
    .await;

    let error = client
        .request(RuntimeRpc::Version)
        .await
        .expect_err("daemon error response should fail");

    assert_error_response(error);
    server.await.expect("server task");
}

macro_rules! typed_helper_tests {
    ($module:ident, $method:ident($($arg:expr),*), $rpc:expr, $ok:expr, $value:expr, $expected:literal) => {
        mod $module {
            use super::*;

            #[tokio::test]
            async fn happy_path_returns_typed_value() {
                let (client, server) = mock_response($rpc, $ok).await;
                let actual = client.$method($($arg),*)
                    .await
                    .expect("typed helper should succeed");
                assert_eq!(actual, $value);
                server.await.expect("server task");
            }

            #[tokio::test]
            async fn daemon_error_maps_to_client_error_response() {
                let (client, server) = mock_response(
                    $rpc,
                    RuntimeResponse::error(ErrorCode::SessionNotFound, "missing session"),
                )
                .await;
                let error = client.$method($($arg),*)
                    .await
                    .expect_err("daemon error should fail");
                assert_error_response(error);
                server.await.expect("server task");
            }

            #[tokio::test]
            async fn unexpected_variant_reports_expected_and_got() {
                let (client, server) = mock_response($rpc, unexpected_for($expected)).await;
                let error = client.$method($($arg),*)
                    .await
                    .expect_err("unexpected response should fail");
                assert_unexpected(error, $expected, unexpected_name_for($expected));
                server.await.expect("server task");
            }
        }
    };
}

fn assert_error_response(error: ClientError) {
    match error {
        ClientError::ErrorResponse { code, message } => {
            assert_eq!(code, ErrorCode::SessionNotFound);
            assert_eq!(message, "missing session");
        }
        other => panic!("unexpected client error: {other:?}"),
    }
}

fn assert_unexpected(error: ClientError, expected: &'static str, got: &'static str) {
    match error {
        ClientError::UnexpectedResponse {
            expected: actual_expected,
            got: actual_got,
        } => {
            assert_eq!(actual_expected, expected);
            assert_eq!(actual_got, got);
        }
        other => panic!("unexpected client error: {other:?}"),
    }
}

fn unexpected_for(expected: &str) -> RuntimeResponse {
    if expected == "Version" {
        RuntimeResponse::Ack
    } else {
        RuntimeResponse::Version(version_payload())
    }
}

fn unexpected_name_for(expected: &str) -> &'static str {
    if expected == "Version" {
        "Ack"
    } else {
        "Version"
    }
}

fn session_id() -> Uuid {
    Uuid::parse_str("018f6e28-0000-7000-8000-000000000001").expect("uuid")
}

fn spawn_request() -> SpawnRequest {
    SpawnRequest {
        session_id: session_id(),
        runtime: RuntimeKind::Claude,
        env: Vec::new(),
        cwd: "/tmp/rtm".into(),
        target: SpawnTarget::Headless(HeadlessSpawnTarget {}),
    }
}

fn spawn_payload() -> SpawnedPayload {
    SpawnedPayload {
        lifecycle: Lifecycle::forking(session_id(), RuntimeKind::Claude),
        event: RuntimeEvent::Lost {
            session_id: session_id(),
            evidence: lilo_rm_core::LostEvidence::PidNotAlive,
        },
        log_dir: None,
        stdout_path: None,
        stderr_path: None,
    }
}

fn kill_request() -> KillRequest {
    KillRequest {
        session_id: session_id(),
        signal: RuntimeSignal::Term,
        grace_secs: 1,
    }
}

fn kill_by_pid_request() -> KillByPidRequest {
    KillByPidRequest {
        pid: 4242,
        signal: 15,
        grace_secs: 1,
    }
}

fn kill_by_pid_response() -> KillByPidResponse {
    KillByPidResponse {
        pid: 4242,
        signal: 15,
        killed_after_grace: false,
    }
}

fn nudge_request() -> NudgeRequest {
    NudgeRequest {
        session_id: session_id(),
        content: "wake up".to_owned(),
    }
}

fn nudge_payload() -> NudgePayload {
    NudgePayload {
        response: NudgeResponse {
            delivered: false,
            outcome: NudgeOutcome::Unsupported(NudgeFailureReason::HeadlessLifecycle),
        },
    }
}

fn capture_request() -> CaptureRequest {
    CaptureRequest {
        session_id: session_id(),
        scrollback_lines: None,
    }
}

fn version_payload() -> VersionPayload {
    VersionPayload {
        version: VersionInfo::new("0.6.0", "test-sha"),
    }
}

fn doctor_payload() -> DoctorPayload {
    DoctorPayload {
        doctor: DoctorResponse {
            version: version_payload().version,
            socket_path: "/tmp/rtmd.sock".to_owned(),
            uptime_secs: 0,
            sqlite: MigrationState {
                applied: 0,
                total: 0,
                applied_descriptions: Vec::new(),
                pending_descriptions: Vec::new(),
            },
            lifecycles: LifecycleCounts::default(),
            watchers: WatcherCounts {
                process_exit_watchers: 0,
                shim_sockets: 0,
                event_waiters: 0,
            },
            launchers: Vec::new(),
            tmux: lilo_rm_core::TmuxStatus {
                available: false,
                version: None,
                error: None,
            },
            log_availability: Vec::new(),
            last_probe_sweep: None,
            recent_lost: Vec::new(),
        },
    }
}

typed_helper_tests!(
    spawn_helper,
    spawn(spawn_request()),
    RuntimeRpc::Spawn {
        request: spawn_request()
    },
    RuntimeResponse::Spawned(spawn_payload()),
    spawn_payload(),
    "Spawned"
);

typed_helper_tests!(
    kill_helper,
    kill(kill_request()),
    RuntimeRpc::Kill {
        request: kill_request()
    },
    RuntimeResponse::Ack,
    (),
    "Ack"
);

typed_helper_tests!(
    kill_by_pid_helper,
    kill_by_pid(kill_by_pid_request()),
    RuntimeRpc::KillByPid {
        request: kill_by_pid_request()
    },
    RuntimeResponse::KillByPid(KillByPidPayload {
        response: kill_by_pid_response()
    }),
    kill_by_pid_response(),
    "KillByPid"
);

typed_helper_tests!(
    status_helper,
    status(StatusFilter::default()),
    RuntimeRpc::Status {
        request: StatusFilter::default().into()
    },
    RuntimeResponse::Status(StatusPayload {
        lifecycles: Vec::new()
    }),
    StatusPayload {
        lifecycles: Vec::new()
    },
    "Status"
);

typed_helper_tests!(
    nudge_helper,
    nudge(nudge_request()),
    RuntimeRpc::Nudge {
        request: nudge_request()
    },
    RuntimeResponse::Nudge(nudge_payload()),
    (),
    "Nudge"
);

typed_helper_tests!(
    capture_helper,
    capture(capture_request()),
    RuntimeRpc::Capture {
        request: capture_request()
    },
    RuntimeResponse::Capture(CapturePayload {
        response: CaptureResponse::Failed(CaptureError::PaneUnavailable)
    }),
    CaptureResponse::Failed(CaptureError::PaneUnavailable),
    "Capture"
);

typed_helper_tests!(
    validate_target_helper,
    validate_target("headless"),
    RuntimeRpc::ValidateTarget {
        request: ValidateTargetRequest {
            target: "headless".to_owned()
        }
    },
    RuntimeResponse::ValidateTarget(ValidateTargetPayload {
        response: ValidateTargetResponse::valid()
    }),
    ValidateTargetResponse::valid(),
    "ValidateTarget"
);

typed_helper_tests!(
    doctor_helper,
    doctor(),
    RuntimeRpc::Doctor,
    RuntimeResponse::Doctor(doctor_payload()),
    doctor_payload(),
    "Doctor"
);

typed_helper_tests!(
    version_helper,
    version(),
    RuntimeRpc::Version,
    RuntimeResponse::Version(version_payload()),
    version_payload(),
    "Version"
);

typed_helper_tests!(
    events_helper,
    events(EventsRequest::default()),
    RuntimeRpc::Events {
        request: EventsRequest::default()
    },
    RuntimeResponse::Events(EventsPayload {
        events: Vec::new(),
        cursor: 0
    }),
    EventBatch::Events {
        events: Vec::new(),
        cursor: 0
    },
    "Events or CursorExpired"
);

#[tokio::test]
async fn events_helper_returns_cursor_expired_batch() {
    let (client, server) = mock_response(
        RuntimeRpc::Events {
            request: EventsRequest {
                since: Some(5),
                wait_ms: Some(0),
            },
        },
        RuntimeResponse::CursorExpired(CursorExpiredPayload { oldest: 9 }),
    )
    .await;

    let actual = client
        .events(EventsRequest {
            since: Some(5),
            wait_ms: Some(0),
        })
        .await
        .expect("cursor expired is a typed event batch");

    assert_eq!(actual, EventBatch::CursorExpired { oldest: 9 });
    server.await.expect("server task");
}
