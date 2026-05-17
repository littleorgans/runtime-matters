mod common;

use common::{RtmHarness, output_stdout, parse_runtime_pid, wait_for_status};
use serde_json::{Value, json};
use uuid::Uuid;

#[test]
fn pass6_mcp_lists_admin_tools_and_reports_status_version_watchers() {
    let harness = RtmHarness::start();
    let tools = mcp_json(&harness, request(1, "tools/list", json!({})));
    let names = tool_names(&tools);
    let generated: Value =
        serde_json::from_str(rtm_cli::generated::mcp_tools::TOOL_LIST_JSON).expect("generated");

    assert_eq!(tools["result"], generated);
    assert_eq!(names, rtm_cli::generated::mcp_tools::TOOL_NAMES);
    assert!(!names.contains(&"spawn"));
    assert!(!names.contains(&"kill"));
    assert!(!names.contains(&"rtm_doctor"));

    let status = call_tool(&harness, 2, "rtm_status", json!({}));
    let status_text = content_text(&status);
    let lifecycles: Vec<Value> = serde_json::from_str(status_text).expect("status rows");
    assert!(lifecycles.is_empty(), "{status_text}");

    let version = call_tool(&harness, 3, "rtm_version", json!({}));
    let version_text: Value = serde_json::from_str(content_text(&version)).expect("version text");
    assert_eq!(version_text["version"], env!("CARGO_PKG_VERSION"));
    assert!(
        version_text["git_sha"]
            .as_str()
            .is_some_and(|sha| !sha.is_empty())
    );

    let watchers = call_tool(&harness, 4, "rtm_watchers", json!({}));
    let watcher_text: Value = serde_json::from_str(content_text(&watchers)).expect("watchers text");
    assert_eq!(watcher_text["kqueue_watchers"], 0);
    assert_eq!(watcher_text["shim_sockets"], 0);

    harness.stop();
}

#[test]
fn pass6_mcp_kill_by_pid_signals_runtime() {
    let harness = RtmHarness::start();
    let session_id = Uuid::now_v7().to_string();
    let spawn = harness.spawn(&session_id);
    assert!(spawn.status.success(), "spawn failed: {spawn:?}");
    let pid = parse_runtime_pid(&output_stdout(spawn));

    let result = call_tool(
        &harness,
        1,
        "rtm_kill_by_pid",
        json!({
            "pid": pid,
            "signal": 15,
            "grace_secs": 0
        }),
    );
    let text: Value = serde_json::from_str(content_text(&result)).expect("kill text");
    assert_eq!(text["pid"], pid);
    assert_eq!(text["signal"], 15);
    wait_for_status(&harness, &session_id, "Exited");

    harness.stop();
}

fn call_tool(harness: &RtmHarness, id: u32, name: &str, arguments: Value) -> Value {
    mcp_json(
        harness,
        request(
            id,
            "tools/call",
            json!({
                "name": name,
                "arguments": arguments
            }),
        ),
    )
}

fn request(id: u32, method: &str, params: Value) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params
    })
    .to_string()
}

fn mcp_json(harness: &RtmHarness, request: String) -> Value {
    let output = harness.mcp_line(&request);
    let success = output.status.success();
    let stdout = String::from_utf8(output.stdout).expect("stdout");
    let stderr = String::from_utf8(output.stderr).expect("stderr");
    assert!(success, "mcp failed: stdout={} stderr={}", stdout, stderr);
    serde_json::from_str(stdout.trim()).expect("mcp json")
}

fn tool_names(response: &Value) -> Vec<&str> {
    response["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool name"))
        .collect()
}

fn content_text(response: &Value) -> &str {
    response["result"]["content"][0]["text"]
        .as_str()
        .expect("content text")
}
