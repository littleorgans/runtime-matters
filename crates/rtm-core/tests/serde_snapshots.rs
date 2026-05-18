use chrono::{TimeZone, Utc};
use lilo_rm_core::{
    KillByPidRequest, KillRequest, LaunchEnv, LaunchSpec, Lifecycle, LostEvidence,
    McpBridgeRequest, NudgeRequest, RuntimeEvent, RuntimeExit, RuntimeKind, RuntimeResponse,
    RuntimeRpc, RuntimeSignal, ShimExit, ShimLaunchRequest, ShimReady, SpawnRequest, SpawnTarget,
    StatusRequest, TerminationEvidence, TmuxSpawnTarget,
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
    let responses = vec![
        RuntimeResponse::Spawned {
            lifecycle,
            event: RuntimeEvent::Running {
                session_id,
                runtime_pid: 4242,
                start_time: timestamp(),
            },
            log_dir: Some("/tmp/rtm/logs/018f6e28-0000-7000-8000-000000000001".into()),
        },
        RuntimeResponse::ShimLaunch {
            launch: LaunchSpec {
                argv: vec!["claude".to_owned(), "--resume".to_owned()],
                env: vec![LaunchEnv::new("RTM", "1")],
                cwd: "/tmp/rtm".into(),
            },
        },
        RuntimeResponse::Ack,
        RuntimeResponse::Stopping,
        RuntimeResponse::Error {
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

fn timestamp() -> chrono::DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap()
}
