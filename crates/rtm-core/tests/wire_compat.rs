mod support;

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use lilo_rm_core::{
    CaptureResponse, CursorExpiredPayload, DoctorPayload, ErrorCode, ErrorPayload, EventsPayload,
    KillByPidResponse, McpBridgePayload, McpBridgeResponse, NudgeOutcome, NudgePayload,
    NudgeResponse, RuntimeEvent, RuntimeResponse, ShimLaunchPayload, SpawnedPayload, StatusPayload,
    ValidateTargetOutcome, ValidateTargetPayload, ValidateTargetResponse, VersionPayload,
    WatchersPayload,
};
use support::{
    doctor_response, headless_lifecycle, launch_spec, pane_snapshot, session_id, test_version_info,
    timestamp, watcher_counts,
};

const FIXTURES: [&str; 16] = [
    "ack.json",
    "capture.json",
    "cursor_expired.json",
    "doctor.json",
    "error.json",
    "events.json",
    "kill_by_pid.json",
    "mcp_bridge.json",
    "nudge.json",
    "shim_launch.json",
    "spawned.json",
    "status.json",
    "stopping.json",
    "validate_target.json",
    "version.json",
    "watchers.json",
];

#[test]
fn runtime_response_v0_5_wire_fixtures_round_trip() {
    assert_fixture_set();

    for (fixture, expected) in expected_responses() {
        let path = fixture_dir().join(fixture);
        let json = fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!("failed to read {}: {error}", path.display());
        });
        let actual: RuntimeResponse = serde_json::from_str(&json).unwrap_or_else(|error| {
            panic!("failed to parse {}: {error}", path.display());
        });

        assert_eq!(actual, expected, "{fixture}");
        assert_eq!(
            serde_json::to_value(&actual).expect("serialize response"),
            serde_json::from_str::<serde_json::Value>(&json).expect("parse fixture value"),
            "{fixture}"
        );
    }
}

fn assert_fixture_set() {
    let actual = fs::read_dir(fixture_dir())
        .expect("fixture dir")
        .map(|entry| {
            let entry = entry.expect("fixture entry");
            entry.file_name().to_string_lossy().into_owned()
        })
        .filter(|name| name.ends_with(".json"))
        .collect::<BTreeSet<_>>();
    let expected = FIXTURES
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<BTreeSet<_>>();

    assert_eq!(actual, expected);
}

fn expected_responses() -> [(&'static str, RuntimeResponse); 16] {
    let session_id = session_id();
    [
        ("ack.json", RuntimeResponse::Ack),
        (
            "capture.json",
            RuntimeResponse::Capture(CaptureResponse::Captured(pane_snapshot())),
        ),
        (
            "cursor_expired.json",
            RuntimeResponse::CursorExpired(CursorExpiredPayload { oldest: 7 }),
        ),
        (
            "doctor.json",
            RuntimeResponse::Doctor(DoctorPayload {
                doctor: doctor_response(),
            }),
        ),
        (
            "error.json",
            RuntimeResponse::Error(ErrorPayload {
                code: ErrorCode::LaunchFailed,
                message: "failed".to_owned(),
            }),
        ),
        (
            "events.json",
            RuntimeResponse::Events(EventsPayload {
                events: vec![RuntimeEvent::Lost {
                    session_id,
                    evidence: lilo_rm_core::LostEvidence::PidNotAlive,
                }],
                cursor: 8,
            }),
        ),
        (
            "kill_by_pid.json",
            RuntimeResponse::KillByPid(KillByPidResponse {
                pid: 4242,
                signal: 15,
                killed_after_grace: false,
            }),
        ),
        (
            "mcp_bridge.json",
            RuntimeResponse::McpBridge(McpBridgePayload {
                response: McpBridgeResponse {
                    line: Some(
                        "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"ok\":true}}".to_owned(),
                    ),
                },
            }),
        ),
        (
            "nudge.json",
            RuntimeResponse::Nudge(NudgePayload {
                response: NudgeResponse {
                    delivered: true,
                    outcome: NudgeOutcome::Delivered,
                },
            }),
        ),
        (
            "shim_launch.json",
            RuntimeResponse::ShimLaunch(ShimLaunchPayload {
                launch: launch_spec(),
            }),
        ),
        (
            "spawned.json",
            RuntimeResponse::Spawned(SpawnedPayload {
                lifecycle: headless_lifecycle(session_id),
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
        ),
        (
            "status.json",
            RuntimeResponse::Status(StatusPayload {
                lifecycles: vec![headless_lifecycle(session_id)],
            }),
        ),
        ("stopping.json", RuntimeResponse::Stopping),
        (
            "validate_target.json",
            RuntimeResponse::ValidateTarget(ValidateTargetPayload {
                response: ValidateTargetResponse {
                    valid: false,
                    outcome: ValidateTargetOutcome::InvalidTarget {
                        message: "invalid spawn target tmux:not-a-pane; expected headless or tmux:<session>:<window>.<pane>"
                            .to_owned(),
                    },
                },
            }),
        ),
        (
            "version.json",
            RuntimeResponse::Version(VersionPayload {
                version: test_version_info(),
            }),
        ),
        (
            "watchers.json",
            RuntimeResponse::Watchers(WatchersPayload {
                watchers: watcher_counts(),
            }),
        ),
    ]
}

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v0_5")
}
