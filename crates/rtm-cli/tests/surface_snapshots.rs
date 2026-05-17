mod common;

use common::mcp::{call_tool, mcp_json, request};
use common::{RtmHarness, output_stdout, spawn_ok};
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
fn mcp_responses_are_stable() {
    let harness = RtmHarness::start();
    let initialize = mcp_json(&harness, request(1, "initialize", json!({})));
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
    let text = serde_json::to_string(structured).expect("structured text");
    response["result"]["content"][0]["text"] = json!(text);
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
