mod common;

use common::mcp::{call_tool, mcp_json, request};
use common::{RtmHarness, output_stdout, spawn_ok};
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
fn doctor_output_is_stable() {
    let harness = RtmHarness::start();
    let output = harness.doctor();
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
