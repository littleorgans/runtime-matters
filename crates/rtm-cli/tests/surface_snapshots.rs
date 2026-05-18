mod common;

use std::process::{Command, Output};

use common::mcp::{call_tool, mcp_json, request};
use common::{RtmHarness, output_stderr, output_stdout, spawn_ok, wait_for_events};
use lilo_rm_core::{RuntimeResponse, RuntimeRpc};
use serde_json::Map;
use serde_json::{Value, json};
use uuid::Uuid;

#[test]
fn status_json_output_is_stable() {
    let harness = RtmHarness::start();
    let session_id = Uuid::now_v7().to_string();
    spawn_ok(&harness, &session_id, "claude");

    let output = harness.status_format(&session_id, "json");
    assert!(output.status.success(), "status json failed: {output:?}");
    let mut rows: Value = serde_json::from_str(&output_stdout(output)).expect("status json");
    redact_lifecycles(&mut rows);

    insta::assert_json_snapshot!(rows);
}

#[test]
fn session_facing_cli_json_outputs_are_stable() {
    let harness = RtmHarness::start();
    let session_id = Uuid::now_v7().to_string();

    let spawn = harness.cli(&[
        "spawn",
        "--runtime",
        "claude",
        "--session-id",
        &session_id,
        "--target",
        "headless",
    ]);
    assert!(spawn.status.success(), "spawn failed: {spawn:?}");

    let nudge = harness.cli(&["nudge", "--session-id", &session_id, "--content", "hello"]);
    assert!(!nudge.status.success(), "nudge unexpectedly succeeded");

    let kill = harness.kill(&session_id, "TERM", 2);
    assert!(kill.status.success(), "kill failed: {kill:?}");
    wait_for_events(&harness, 2);

    let mut version = json_stdout(harness.cli(&["version"]));
    let mut doctor = json_stdout(harness.cli(&["doctor"]));
    let mut status = json_stdout(harness.cli(&["status", "--session-id", &session_id]));
    let mut events = json_stdout(harness.cli(&["events"]));
    let mut spawn = json_from_output(&output_stdout(spawn));
    let mut kill = json_from_output(&output_stdout(kill));
    let mut nudge_error = json_from_output(&output_stderr(nudge));

    redact_version(&mut version);
    redact_doctor_json(&mut doctor);
    redact_lifecycles(&mut status);
    redact_events(&mut events);
    redact_spawn(&mut spawn);
    redact_kill(&mut kill);
    redact_error(&mut nudge_error);

    insta::assert_json_snapshot!(json!({
        "version": version,
        "doctor": doctor,
        "status": status,
        "events": events,
        "spawn": spawn,
        "kill": kill,
        "nudge_error": nudge_error,
    }));
}

#[test]
fn doctor_output_is_stable() {
    let harness = RtmHarness::start();
    let output = harness.cli(&["doctor", "--format", "human"]);
    assert!(output.status.success(), "doctor failed: {output:?}");

    insta::assert_snapshot!(normalize_doctor(&output_stdout(output)));
}

#[test]
fn doctor_json_response_is_stable() {
    let harness = RtmHarness::start();
    let response = tokio::runtime::Runtime::new()
        .expect("tokio runtime")
        .block_on(rtm_cli::shared::request(
            harness.socket_path(),
            RuntimeRpc::Doctor,
        ))
        .expect("doctor rpc");

    let RuntimeResponse::Doctor { doctor } = response else {
        panic!("unexpected doctor response: {response:?}");
    };
    let mut doctor = serde_json::to_value(doctor).expect("doctor json");
    redact_doctor_json(&mut doctor);

    insta::assert_json_snapshot!(doctor);
}

#[test]
fn mcp_responses_are_stable() {
    let harness = RtmHarness::start();
    let mut initialize = mcp_json(&harness, request(1, "initialize", json!({})));
    redact_initialize(&mut initialize);
    let tools = mcp_json(&harness, request(2, "tools/list", json!({})));
    let status = call_tool(&harness, 3, "rtm_status", json!({}));
    let mut version = call_tool(&harness, 4, "rtm_version", json!({}));
    let watchers = call_tool(&harness, 5, "rtm_watchers", json!({}));

    let session_id = Uuid::now_v7().to_string();
    let spawn = spawn_ok(&harness, &session_id, "claude");
    let runtime_pid = common::parse_runtime_pid(&spawn);
    let mut kill = call_tool(
        &harness,
        6,
        "rtm_kill_by_pid",
        json!({
            "pid": runtime_pid,
            "signal": 15,
            "grace_secs": 0
        }),
    );
    redact_tool_payload(&mut version);
    redact_tool_payload(&mut kill);

    insta::assert_json_snapshot!(json!({
        "initialize": initialize,
        "tools": tools,
        "status": status,
        "version": version,
        "watchers": watchers,
        "kill_by_pid": kill,
    }));
}

fn redact_lifecycles(rows: &mut Value) {
    let Some(rows) = rows.as_array_mut() else {
        return;
    };
    for row in rows {
        row["session_id"] = json!("[uuid]");
        row["shim_pid"] = json!("[pid]");
        row["runtime_pid"] = json!("[pid]");
        row["start_time"] = json!("[timestamp]");
        row["tmux_pane"] = json!("[tmux_pane]");
    }
}

fn redact_version(version: &mut Value) {
    version["version"] = json!("[version]");
    version["git_sha"] = json!("[git_sha]");
}

fn redact_events(events: &mut Value) {
    let Some(events) = events.as_array_mut() else {
        return;
    };
    for event in events {
        event["payload"]["session_id"] = json!("[uuid]");
        if event["payload"].get("runtime_pid").is_some() {
            event["payload"]["runtime_pid"] = json!("[pid]");
        }
        if event["payload"].get("start_time").is_some() {
            event["payload"]["start_time"] = json!("[timestamp]");
        }
    }
}

fn redact_spawn(spawn: &mut Value) {
    let payload = &mut spawn["payload"];
    payload["lifecycle"]["session_id"] = json!("[uuid]");
    payload["lifecycle"]["shim_pid"] = json!("[pid]");
    payload["lifecycle"]["runtime_pid"] = json!("[pid]");
    payload["lifecycle"]["start_time"] = json!("[timestamp]");
    payload["event"]["payload"]["session_id"] = json!("[uuid]");
    payload["event"]["payload"]["runtime_pid"] = json!("[pid]");
    payload["event"]["payload"]["start_time"] = json!("[timestamp]");
    payload["log_dir"] = json!("[path]");
    payload["stdout_path"] = json!("[path]");
    payload["stderr_path"] = json!("[path]");
}

fn redact_kill(kill: &mut Value) {
    kill["session_id"] = json!("[uuid]");
}

fn redact_error(error: &mut Value) {
    error["message"] = json!("[message]");
    if let Some(details) = error.get_mut("details")
        && let Some(causes) = details.get_mut("causes").and_then(Value::as_array_mut)
    {
        for cause in causes {
            *cause = json!("[cause]");
        }
    }
}

fn json_stdout(output: Output) -> Value {
    assert!(output.status.success(), "json command failed: {output:?}");
    json_from_output(&output_stdout(output))
}

fn json_from_output(output: &str) -> Value {
    jq_round_trip(output);
    serde_json::from_str(output).expect("json output")
}

fn jq_round_trip(output: &str) {
    let jq = Command::new("jq")
        .arg("-c")
        .arg(".")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    let Ok(mut jq) = jq else {
        return;
    };
    let mut stdin = jq.stdin.take().expect("jq stdin");
    std::io::Write::write_all(&mut stdin, output.as_bytes()).expect("write jq stdin");
    drop(stdin);
    assert!(jq.wait().expect("jq wait").success(), "jq rejected output");
}

fn redact_tool_payload(response: &mut Value) {
    let structured = &mut response["result"]["structuredContent"];
    if structured.get("git_sha").is_some() {
        structured["git_sha"] = json!("[git_sha]");
    }
    if structured.get("pid").is_some() {
        structured["pid"] = json!("[pid]");
    }
    if structured.get("version").is_some() {
        structured["version"] = json!("[version]");
    }
    let text = serde_json::to_string(structured).expect("structured text");
    response["result"]["content"][0]["text"] = json!(text);
}

fn redact_initialize(response: &mut Value) {
    let server_info = &mut response["result"]["serverInfo"];
    if server_info.get("version").is_some() {
        server_info["version"] = json!("[version]");
    }
}

fn redact_doctor_json(doctor: &mut Value) {
    {
        let fields = object_mut(doctor, "doctor");
        redact_field(fields, "socket_path", "[socket]");
        redact_field(fields, "uptime_secs", "[uptime]");
        redact_field(fields, "last_probe_sweep", "[timestamp]");
    }
    redact_nested_field(doctor, "version", "version", "[version]");
    redact_nested_field(doctor, "version", "git_sha", "[git_sha]");
    for field in ["forking", "running", "exited", "lost"] {
        redact_nested_field(doctor, "lifecycles", field, "[count]");
    }
    for field in ["kqueue_watchers", "shim_sockets"] {
        redact_nested_field(doctor, "watchers", field, "[count]");
    }
    for field in ["available", "version", "error"] {
        redact_nested_field(doctor, "tmux", field, "[tmux]");
    }
    redact_array_field(doctor, "launchers", "command", "[command]");
    redact_array_field(doctor, "launchers", "error", "[launcher_error]");
}

fn redact_nested_field(root: &mut Value, object: &str, field: &str, replacement: &str) {
    let child = field_mut(root, object);
    redact_field(object_mut(child, object), field, replacement);
}

fn redact_array_field(root: &mut Value, array: &str, field: &str, replacement: &str) {
    let values = field_mut(root, array)
        .as_array_mut()
        .unwrap_or_else(|| panic!("doctor field {array} must be an array"));
    for value in values {
        redact_field(object_mut(value, array), field, replacement);
    }
}

fn field_mut<'a>(root: &'a mut Value, field: &str) -> &'a mut Value {
    object_mut(root, "doctor")
        .get_mut(field)
        .unwrap_or_else(|| panic!("missing doctor field {field}"))
}

fn object_mut<'a>(value: &'a mut Value, label: &str) -> &'a mut Map<String, Value> {
    value
        .as_object_mut()
        .unwrap_or_else(|| panic!("{label} must be an object"))
}

fn redact_field(fields: &mut Map<String, Value>, field: &str, replacement: &str) {
    let value = fields
        .get_mut(field)
        .unwrap_or_else(|| panic!("missing doctor field {field}"));
    *value = json!(replacement);
}

fn normalize_doctor(output: &str) -> String {
    output
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if trimmed.starts_with("version") {
                "  version             [version]".to_owned()
            } else if trimmed.starts_with("socket") {
                "  socket              [socket]".to_owned()
            } else if trimmed.starts_with("uptime") {
                "  uptime              [uptime]".to_owned()
            } else if trimmed.starts_with("claude") {
                "  claude              [command]".to_owned()
            } else if trimmed.starts_with("codex") {
                "  codex               [command]".to_owned()
            } else if trimmed.starts_with("tmux") {
                "tmux                  [availability]".to_owned()
            } else if trimmed.starts_with("last probe sweep") {
                "last probe sweep      [timestamp]".to_owned()
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
