mod common;

use std::io::BufReader;
use std::os::unix::net::UnixStream;

use common::{RtmHarness, output_stderr, output_stdout, spawn_ok, wait_for_log};
use lilo_rm_core::{
    ErrorCode, HeadlessSpawnTarget, LaunchEnv, NudgeFailureReason, NudgeOutcome, NudgePayload,
    NudgeRequest, NudgeResponse, RuntimeKind, RuntimeResponse, RuntimeRpc, SpawnRequest,
    SpawnTarget, ValidateTargetOutcome, ValidateTargetPayload, ValidateTargetRequest,
    ValidateTargetResponse, read_json_line_blocking, write_json_line_blocking,
};
use serde_json::Value;
use uuid::Uuid;

#[test]
fn explicit_headless_spawn_records_no_tmux_pane_and_rejects_nudge() {
    let harness = RtmHarness::start();
    let session_id = Uuid::now_v7().to_string();
    spawn_ok(&harness, &session_id, "claude");

    let status = harness.status_format(&session_id, "json");
    assert!(status.status.success(), "status failed: {status:?}");
    let lifecycles: Value = serde_json::from_str(&output_stdout(status)).expect("status json");
    assert_eq!(lifecycles[0]["tmux_pane"], Value::Null);

    let response = request_raw(
        &harness,
        RuntimeRpc::Nudge {
            request: NudgeRequest {
                session_id: session_id.parse().expect("session id"),
                content: "headless".to_owned(),
            },
        },
    );
    assert_eq!(
        response,
        RuntimeResponse::Nudge(NudgePayload {
            response: NudgeResponse {
                delivered: false,
                outcome: NudgeOutcome::Unsupported(NudgeFailureReason::HeadlessLifecycle),
            },
        })
    );

    let nudge = harness.nudge(&session_id, "headless");
    assert!(!nudge.status.success(), "nudge succeeded: {nudge:?}");
    let stderr = output_stderr(nudge);
    assert!(
        stderr.contains(&format!(
            "nudge unsupported; reason=headless_lifecycle session_id={session_id}"
        )),
        "{stderr}"
    );
}

#[test]
fn missing_session_nudge_uses_structured_error_code() {
    let harness = RtmHarness::start();
    let session_id = Uuid::now_v7();

    let response = request_raw(
        &harness,
        RuntimeRpc::Nudge {
            request: NudgeRequest {
                session_id,
                content: "missing".to_owned(),
            },
        },
    );
    let RuntimeResponse::Error(payload) = response else {
        panic!("unexpected missing-session response: {response:?}");
    };
    assert_eq!(payload.code, ErrorCode::SessionNotFound);
    assert!(
        payload
            .message
            .contains(&format!("session {session_id} not found"))
    );
}

#[test]
fn headless_spawn_pipes_stdout_and_stderr_to_session_logs() {
    let harness = RtmHarness::start_outside_tmux();
    let session_id = Uuid::now_v7();
    let response = tokio::runtime::Runtime::new()
        .expect("tokio runtime")
        .block_on(rtm_cli::shared::request(
            harness.socket_path(),
            RuntimeRpc::Spawn {
                request: SpawnRequest {
                    session_id,
                    runtime: RuntimeKind::Claude,
                    env: vec![LaunchEnv::new("RTM_TEST_STDIO_SENTINELS", "1")],
                    cwd: harness.rtm_home().to_path_buf(),
                    target: SpawnTarget::Headless(HeadlessSpawnTarget {}),
                },
            },
        ))
        .expect("headless spawn");

    let RuntimeResponse::Spawned(payload) = response else {
        panic!("unexpected spawn response: {response:?}");
    };
    let log_dir = payload.log_dir.expect("headless log dir");
    let stdout_path = payload.stdout_path.expect("headless stdout path");
    let stderr_path = payload.stderr_path.expect("headless stderr path");
    assert_eq!(payload.lifecycle.tmux_pane, None);
    assert_eq!(
        log_dir,
        harness.rtm_home().join("logs").join(session_id.to_string())
    );
    assert_eq!(stdout_path, log_dir.join("stdout.log"));
    assert_eq!(stderr_path, log_dir.join("stderr.log"));
    wait_for_log(stdout_path, "HELLO\n");
    wait_for_log(stderr_path, "WORLD\n");
}

#[test]
fn validate_target_rpc_reports_headless_and_parse_outcomes() {
    let harness = RtmHarness::start();

    assert_eq!(
        validate_target(&harness, "headless"),
        RuntimeResponse::ValidateTarget(ValidateTargetPayload {
            response: ValidateTargetResponse::valid(),
        })
    );

    let RuntimeResponse::ValidateTarget(payload) = validate_target(&harness, "tmux:not-a-pane")
    else {
        panic!("expected validate target response");
    };
    let response = payload.response;
    assert!(!response.valid);
    assert!(matches!(
        response.outcome,
        ValidateTargetOutcome::InvalidTarget { .. }
    ));

    assert_eq!(
        validate_target(&harness, "ssh:remote"),
        RuntimeResponse::ValidateTarget(ValidateTargetPayload {
            response: ValidateTargetResponse::unsupported_target("ssh:remote"),
        })
    );
}

#[test]
fn validate_target_rpc_checks_tmux_liveness_when_available() {
    let Some(tmux_session) = common::tmux::TmuxSession::start("rtm-validate-target") else {
        eprintln!("skipping tmux validate target test because tmux is unavailable");
        return;
    };
    let harness = RtmHarness::start();
    let target = format!("tmux:{}", tmux_session.pane());

    assert_eq!(
        validate_target(&harness, &target),
        RuntimeResponse::ValidateTarget(ValidateTargetPayload {
            response: ValidateTargetResponse::valid(),
        })
    );

    let address = tmux_session.pane();
    tmux_session.kill();

    assert_eq!(
        validate_target(&harness, &target),
        RuntimeResponse::ValidateTarget(ValidateTargetPayload {
            response: ValidateTargetResponse::tmux_pane_dead(
                address.parse().expect("tmux address"),
            ),
        })
    );
}

fn validate_target(harness: &RtmHarness, target: &str) -> RuntimeResponse {
    request_raw(
        harness,
        RuntimeRpc::ValidateTarget {
            request: ValidateTargetRequest {
                target: target.to_owned(),
            },
        },
    )
}

fn request_raw(harness: &RtmHarness, rpc: RuntimeRpc) -> RuntimeResponse {
    let mut stream = UnixStream::connect(harness.socket_path()).expect("connect daemon");
    write_json_line_blocking(&mut stream, &rpc).expect("write request");
    let mut reader = BufReader::new(stream);
    read_json_line_blocking(&mut reader).expect("read response")
}
