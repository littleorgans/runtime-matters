mod support;

use lilo_rm_core::{
    CaptureError, CaptureRequest, CaptureResponse, CursorExpiredPayload, DoctorPayload, ErrorCode,
    ErrorPayload, EventsPayload, EventsRequest, KillByPidRequest, KillRequest, LogAvailability,
    LogsUnavailableReason, LostEvidence, McpBridgeRequest, NudgeFailureReason, NudgeOutcome,
    NudgePayload, NudgeRequest, NudgeResponse, RuntimeEvent, RuntimeExit, RuntimeKind,
    RuntimeResponse, RuntimeRpc, RuntimeSignal, ShimExit, ShimLaunchPayload, ShimLaunchRequest,
    SpawnRequest, SpawnTarget, SpawnedPayload, StatusRequest, TerminationEvidence, TmuxSpawnTarget,
    ValidateTargetOutcome, ValidateTargetPayload, ValidateTargetRequest, ValidateTargetResponse,
    VersionPayload,
};
use serde_json::json;
use support::{
    doctor_response, headless_lifecycle, launch_spec, other_session_id, pane_snapshot, ready,
    session_id, test_version_info as version_info, timestamp, tmux_lifecycle,
};

#[test]
fn runtime_rpc_json_shapes_are_stable() {
    let session_id = session_id();
    let ready = ready(session_id);
    let rpcs = vec![
        RuntimeRpc::Spawn {
            request: SpawnRequest {
                session_id,
                runtime: RuntimeKind::Claude,
                env: Vec::new(),
                cwd: "/tmp/rtm".into(),
                target: SpawnTarget::Tmux(TmuxSpawnTarget {
                    address: "rtm:0.1".parse().expect("address"),
                }),
            },
        },
        RuntimeRpc::ValidateTarget {
            request: ValidateTargetRequest {
                target: "tmux:rtm:0.1".to_owned(),
            },
        },
        RuntimeRpc::Kill {
            request: KillRequest {
                session_id,
                signal: RuntimeSignal::Term,
                grace_secs: 2,
            },
        },
        RuntimeRpc::KillByPid {
            request: KillByPidRequest {
                pid: 4242,
                signal: 15,
                grace_secs: 1,
            },
        },
        RuntimeRpc::Nudge {
            request: NudgeRequest {
                session_id,
                content: "wake up".to_owned(),
            },
        },
        RuntimeRpc::Capture {
            request: CaptureRequest {
                session_id,
                scrollback_lines: Some(500),
            },
        },
        RuntimeRpc::Status {
            request: StatusRequest {
                session_id: Some(session_id),
                session_ids: vec![other_session_id()],
                updated_since: Some(timestamp()),
                runtime: Some("claude".to_owned()),
                state: Some("Running".to_owned()),
            },
        },
        RuntimeRpc::Version,
        RuntimeRpc::Watchers,
        RuntimeRpc::Doctor,
        RuntimeRpc::Events {
            request: EventsRequest {
                since: Some(7),
                wait_ms: Some(500),
            },
        },
        RuntimeRpc::Stop,
        RuntimeRpc::McpBridge {
            request: McpBridgeRequest {
                line: "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}".to_owned(),
            },
        },
        RuntimeRpc::ShimLaunch {
            request: ShimLaunchRequest { session_id },
        },
        RuntimeRpc::ShimReady { ready },
        RuntimeRpc::ShimExit {
            exit: ShimExit {
                session_id,
                exit: RuntimeExit::new(None, Some(9)),
            },
        },
    ];

    insta::assert_json_snapshot!(rpcs);
}

#[test]
fn runtime_event_json_shapes_are_stable() {
    let session_id = session_id();
    let events = vec![
        RuntimeEvent::Running {
            session_id,
            runtime_pid: 4242,
            start_time: timestamp(),
        },
        RuntimeEvent::Terminated {
            session_id,
            exit_code: None,
            signal: Some(9),
            evidence: TerminationEvidence::ProcessExit,
        },
        RuntimeEvent::Lost {
            session_id,
            evidence: LostEvidence::PidReuseDetected,
        },
    ];

    insta::assert_json_snapshot!(events);
}

#[test]
fn runtime_response_json_shapes_are_stable() {
    let session_id = session_id();
    let lifecycle = headless_lifecycle(session_id);
    let tmux_lifecycle = tmux_lifecycle(session_id);
    let responses = vec![
        RuntimeResponse::Spawned(SpawnedPayload {
            lifecycle,
            event: RuntimeEvent::Running {
                session_id,
                runtime_pid: 4242,
                start_time: timestamp(),
            },
            log_dir: Some("/tmp/rtm/logs/018f6e28-0000-7000-8000-000000000001".into()),
            stdout_path: Some(
                "/tmp/rtm/logs/018f6e28-0000-7000-8000-000000000001/stdout.log".into(),
            ),
            stderr_path: Some(
                "/tmp/rtm/logs/018f6e28-0000-7000-8000-000000000001/stderr.log".into(),
            ),
        }),
        RuntimeResponse::Spawned(SpawnedPayload {
            lifecycle: tmux_lifecycle,
            event: RuntimeEvent::Running {
                session_id,
                runtime_pid: 4243,
                start_time: timestamp(),
            },
            log_dir: None,
            stdout_path: None,
            stderr_path: None,
        }),
        RuntimeResponse::ValidateTarget(ValidateTargetPayload {
            response: ValidateTargetResponse::valid(),
        }),
        RuntimeResponse::ValidateTarget(ValidateTargetPayload {
            response: ValidateTargetResponse {
                valid: false,
                outcome: ValidateTargetOutcome::InvalidTarget {
                    message: "invalid spawn target tmux:not-a-pane; expected headless or tmux:<session>:<window>.<pane>"
                        .to_owned(),
                },
            },
        }),
        RuntimeResponse::ValidateTarget(ValidateTargetPayload {
            response: ValidateTargetResponse::tmux_pane_dead(
                "rtm:0.1".parse().expect("tmux address"),
            ),
        }),
        RuntimeResponse::ValidateTarget(ValidateTargetPayload {
            response: ValidateTargetResponse::unsupported_target("ssh:remote"),
        }),
        RuntimeResponse::ShimLaunch(ShimLaunchPayload {
            launch: launch_spec(),
        }),
        RuntimeResponse::Ack,
        RuntimeResponse::Nudge(NudgePayload {
            response: NudgeResponse {
                delivered: true,
                outcome: NudgeOutcome::Delivered,
            },
        }),
        RuntimeResponse::Nudge(NudgePayload {
            response: NudgeResponse {
                delivered: false,
                outcome: NudgeOutcome::Unsupported(NudgeFailureReason::HeadlessLifecycle),
            },
        }),
        RuntimeResponse::Nudge(NudgePayload {
            response: NudgeResponse {
                delivered: false,
                outcome: NudgeOutcome::Failed(NudgeFailureReason::TmuxPaneDead),
            },
        }),
        RuntimeResponse::Capture(CaptureResponse::Captured(pane_snapshot())),
        RuntimeResponse::Capture(CaptureResponse::Failed(CaptureError::PaneUnavailable)),
        RuntimeResponse::Version(VersionPayload {
            version: version_info(),
        }),
        RuntimeResponse::Doctor(DoctorPayload {
            doctor: doctor_response(),
        }),
        RuntimeResponse::Stopping,
        RuntimeResponse::Error(ErrorPayload {
            code: ErrorCode::LaunchFailed,
            message: "failed".to_owned(),
        }),
        RuntimeResponse::CursorExpired(CursorExpiredPayload { oldest: 7 }),
        RuntimeResponse::Events(EventsPayload {
            events: vec![RuntimeEvent::Lost {
                session_id,
                evidence: LostEvidence::PidNotAlive,
            }],
            cursor: 8,
        }),
    ];

    insta::assert_json_snapshot!(responses);
}

#[test]
fn log_availability_json_shapes_are_stable() {
    let values = vec![
        LogAvailability::Headless {
            stdout_path: "/tmp/rtm/stdout.log".into(),
            stderr_path: "/tmp/rtm/stderr.log".into(),
        },
        LogAvailability::TmuxPaneSnapshot,
        LogAvailability::Unavailable {
            reason: LogsUnavailableReason::TmuxTarget,
        },
    ];

    insta::assert_json_snapshot!(values);
}

#[test]
fn pane_snapshot_json_shape_is_stable() {
    insta::assert_json_snapshot!(pane_snapshot());
}

#[test]
fn capture_error_json_names_are_stable() {
    let errors = vec![
        CaptureError::NotATmuxTarget,
        CaptureError::PaneUnavailable,
        CaptureError::SessionMissing,
        CaptureError::TmuxNotAvailable,
        CaptureError::CapturePaneFailed {
            stderr: "no pane".to_owned(),
        },
    ];

    insta::assert_json_snapshot!(errors);
}

#[test]
fn runtime_capability_json_names_are_stable() {
    insta::assert_json_snapshot!(lilo_rm_core::RUNTIME_PROTOCOL_CAPABILITIES);
}

#[test]
fn error_code_json_names_are_stable() {
    let codes = vec![
        ErrorCode::RuntimeUnavailable,
        ErrorCode::SessionNotFound,
        ErrorCode::TmuxPaneDead,
        ErrorCode::HeadlessNudgeUnsupported,
        ErrorCode::LaunchFailed,
        ErrorCode::InvalidTarget,
        ErrorCode::ProtocolMismatch,
    ];

    insta::assert_json_snapshot!(codes);
}

#[test]
fn status_request_accepts_legacy_single_session_id() {
    let request = serde_json::from_value::<StatusRequest>(json!({
        "session_id": session_id(),
        "runtime": "claude",
        "state": "Running"
    }))
    .expect("legacy status request");

    assert_eq!(request.session_id, Some(session_id()));
    assert!(request.session_ids.is_empty());
    assert_eq!(request.updated_since, None);
}

#[test]
fn spawn_request_json_requires_target() {
    let error = serde_json::from_value::<SpawnRequest>(json!({
        "session_id": session_id(),
        "runtime": "claude",
        "env": [],
        "cwd": "/tmp/rtm"
    }))
    .expect_err("spawn request without target should fail");

    assert!(
        error.to_string().contains("missing field `target`"),
        "{error}"
    );
}

#[test]
fn spawn_request_json_requires_cwd() {
    let error = serde_json::from_value::<SpawnRequest>(json!({
        "session_id": session_id(),
        "runtime": "claude",
        "env": [],
        "target": { "type": "headless", "payload": {} }
    }))
    .expect_err("spawn request without cwd should fail");

    assert!(error.to_string().contains("missing field `cwd`"), "{error}");
}
