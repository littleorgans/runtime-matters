mod common;

use std::fs::{OpenOptions, create_dir_all};
use std::io::Write;
use std::time::Duration;

use common::{RtmHarness, output_stdout, spawn_ok, wait_until};
use lilo_rm_core::{EventsRequest, RuntimeResponse, RuntimeRpc};
use serde_json::json;
use uuid::Uuid;

const FIRST_SESSION: &str = "018f6e28-0000-7000-8000-000000000101";
const SECOND_SESSION: &str = "018f6e28-0000-7000-8000-000000000102";

#[test]
fn events_resume_after_daemon_restart_without_duplication() {
    let mut harness = RtmHarness::start();
    spawn_ok(&harness, FIRST_SESSION, "claude");
    let cursor = wait_for_rpc_events(&harness, None, 1).cursor();

    harness.stop_rtmd();
    harness.start_rtmd();
    spawn_ok(&harness, SECOND_SESSION, "claude");

    let resumed = wait_for_rpc_events(&harness, Some(cursor), 1);

    let RuntimeResponse::Events { events, cursor: _ } = resumed else {
        panic!("expected events response");
    };
    assert_eq!(events.len(), 1);
    assert!(
        matches!(events[0], lilo_rm_core::RuntimeEvent::Running { .. }),
        "{events:?}"
    );
    harness.stop();
}

#[test]
fn cli_events_since_matches_rpc_cursor_filter() {
    let harness = RtmHarness::start();
    spawn_ok(&harness, FIRST_SESSION, "claude");
    let cursor = wait_for_rpc_events(&harness, None, 1).cursor();
    spawn_ok(&harness, SECOND_SESSION, "claude");
    let rpc = wait_for_rpc_events(&harness, Some(cursor), 1);

    let output = harness.events_since(cursor);
    assert!(output.status.success(), "events --since failed: {output:?}");
    let stdout = output_stdout(output);

    let RuntimeResponse::Events { events, cursor: _ } = rpc else {
        panic!("expected events response");
    };
    assert_eq!(events.len(), stdout.lines().count());
    assert!(stdout.contains(SECOND_SESSION), "{stdout}");
    assert!(!stdout.contains(FIRST_SESSION), "{stdout}");
    harness.stop();
}

#[test]
fn expired_cursor_returns_cursor_expired_frame() {
    let mut harness = RtmHarness::start();
    harness.stop_rtmd();
    write_event_log(&harness, &[event_record(3, FIRST_SESSION)], "");
    harness.start_rtmd();

    let response = rpc_events(&harness, Some(0));

    assert_eq!(response, RuntimeResponse::CursorExpired { oldest: 2 });
    harness.stop();
}

#[test]
fn startup_recovery_drops_trailing_partial_event_line() {
    let mut harness = RtmHarness::start();
    harness.stop_rtmd();
    write_event_log(&harness, &[event_record(1, FIRST_SESSION)], r#"{"seq":2"#);
    harness.start_rtmd();

    let response = rpc_events(&harness, Some(0));

    let RuntimeResponse::Events { events, cursor } = response else {
        panic!("expected events response");
    };
    assert_eq!(events.len(), 1);
    assert_eq!(cursor, 1);
    harness.stop();
}

trait Cursor {
    fn cursor(&self) -> u64;
}

impl Cursor for RuntimeResponse {
    fn cursor(&self) -> u64 {
        match self {
            RuntimeResponse::Events { cursor, .. } => *cursor,
            other => panic!("expected events response, got {other:?}"),
        }
    }
}

fn wait_for_rpc_events(
    harness: &RtmHarness,
    since: Option<u64>,
    expected: usize,
) -> RuntimeResponse {
    wait_until(Duration::from_secs(5), || {
        let response = rpc_events(harness, since);
        match &response {
            RuntimeResponse::Events { events, .. } if events.len() == expected => Some(response),
            _ => None,
        }
    })
    .unwrap_or_else(|| panic!("events never reached {expected}"))
}

fn rpc_events(harness: &RtmHarness, since: Option<u64>) -> RuntimeResponse {
    tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(lilo_rm_client::request(
            harness.socket_path(),
            RuntimeRpc::Events {
                request: EventsRequest { since },
            },
        ))
        .expect("events rpc")
}

fn write_event_log(harness: &RtmHarness, records: &[serde_json::Value], tail: &str) {
    let path = harness
        .db_path()
        .parent()
        .expect("db parent")
        .join("events.jsonl");
    create_dir_all(path.parent().expect("event log parent")).expect("event log dir");
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&path)
        .expect("event log");
    for record in records {
        writeln!(file, "{record}").expect("record");
    }
    if !tail.is_empty() {
        file.write_all(tail.as_bytes()).expect("tail");
    }
}

fn event_record(seq: u64, session_id: &str) -> serde_json::Value {
    json!({
        "seq": seq,
        "ts_ms": 1_700_000_000_000_u64,
        "kind": "running",
        "payload": {
            "session_id": Uuid::parse_str(session_id).expect("session id"),
            "runtime_pid": 4242,
            "start_time": "2023-11-14T22:13:20Z"
        }
    })
}
