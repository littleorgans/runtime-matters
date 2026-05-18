use chrono::{TimeZone, Utc};
use lilo_rm_core::{
    ErrorCode, KillByPidRequest, KillRequest, LaunchEnv, LaunchSpec, Lifecycle, LostEvidence,
    McpBridgeRequest, NudgeFailureReason, NudgeOutcome, NudgeRequest, NudgeResponse, RuntimeEvent,
    RuntimeExit, RuntimeKind, RuntimeResponse, RuntimeRpc, RuntimeSignal, ShimExit,
    ShimLaunchRequest, ShimReady, SpawnRequest, SpawnTarget, StatusRequest, TerminationEvidence,
    TmuxSpawnTarget,
};
use serde_json::json;
use uuid::Uuid;

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
        RuntimeRpc::Events,
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
            evidence: TerminationEvidence::KqueueExit,
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
    let mut lifecycle = Lifecycle::forking(session_id, RuntimeKind::Claude);
    assert!(lifecycle.mark_running(ready(session_id)));
    let mut tmux_lifecycle = Lifecycle::forking(session_id, RuntimeKind::Claude);
    assert!(tmux_lifecycle.mark_running(ready(session_id)));
    let responses = vec![
        RuntimeResponse::Spawned {
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
        },
        RuntimeResponse::Spawned {
            lifecycle: tmux_lifecycle,
            event: RuntimeEvent::Running {
                session_id,
                runtime_pid: 4243,
                start_time: timestamp(),
            },
            log_dir: None,
            stdout_path: None,
            stderr_path: None,
        },
        RuntimeResponse::ShimLaunch {
            launch: LaunchSpec {
                argv: vec!["claude".to_owned(), "--resume".to_owned()],
                env: vec![LaunchEnv::new("RTM", "1")],
                cwd: "/tmp/rtm".into(),
            },
        },
        RuntimeResponse::Ack,
        RuntimeResponse::Nudge {
            response: NudgeResponse {
                delivered: true,
                outcome: NudgeOutcome::Delivered,
            },
        },
        RuntimeResponse::Nudge {
            response: NudgeResponse {
                delivered: false,
                outcome: NudgeOutcome::Unsupported(NudgeFailureReason::HeadlessLifecycle),
            },
        },
        RuntimeResponse::Nudge {
            response: NudgeResponse {
                delivered: false,
                outcome: NudgeOutcome::Failed(NudgeFailureReason::TmuxPaneDead),
            },
        },
        RuntimeResponse::Stopping,
        RuntimeResponse::Error {
            code: ErrorCode::LaunchFailed,
            message: "failed".to_owned(),
        },
        RuntimeResponse::Events {
            events: vec![RuntimeEvent::Lost {
                session_id,
                evidence: LostEvidence::PidNotAlive,
            }],
        },
    ];

    insta::assert_json_snapshot!(responses);
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

fn ready(session_id: Uuid) -> ShimReady {
    ShimReady {
        session_id,
        shim_pid: 4241,
        runtime_pid: 4242,
        start_time: timestamp(),
        tmux_pane: Some("rtm:0.1".parse().expect("pane")),
    }
}

fn session_id() -> Uuid {
    Uuid::parse_str("018f6e28-0000-7000-8000-000000000001").expect("uuid")
}

fn other_session_id() -> Uuid {
    Uuid::parse_str("018f6e28-0000-7000-8000-000000000002").expect("uuid")
}

fn timestamp() -> chrono::DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap()
}
