use std::process::Output;

use super::RtmHarness;

pub fn output_stdout(output: Output) -> String {
    String::from_utf8(output.stdout).expect("stdout")
}

pub fn output_stderr(output: Output) -> String {
    String::from_utf8(output.stderr).expect("stderr")
}

pub fn parse_runtime_pid(stdout: &str) -> u32 {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(stdout) {
        return value["payload"]["lifecycle"]["runtime_pid"]
            .as_u64()
            .expect("runtime pid in spawn json") as u32;
    }
    stdout
        .split("runtime_pid=")
        .nth(1)
        .and_then(|tail| tail.split_whitespace().next())
        .and_then(|value| value.parse().ok())
        .expect("runtime pid in spawn output")
}

pub fn parse_status_pid(stdout: &str) -> u32 {
    stdout.trim().parse().unwrap_or_else(|_| {
        let value: serde_json::Value = serde_json::from_str(stdout).expect("status json");
        value[0]["runtime_pid"].as_u64().expect("status pid") as u32
    })
}

pub fn spawn_ok(harness: &RtmHarness, session_id: &str, runtime: &str) -> String {
    let output = harness.spawn_runtime(session_id, runtime);
    spawn_output_ok(output, runtime)
}

pub fn spawn_output_ok(output: Output, runtime: &str) -> String {
    assert!(
        output.status.success(),
        "{runtime} spawn failed: {output:?}"
    );
    output_stdout(output)
}

pub fn status_pid(harness: &RtmHarness, session_id: &str, field: &str) -> u32 {
    let output = harness.status_format(session_id, "json");
    assert!(output.status.success(), "status failed: {output:?}");
    status_json_pid(&output_stdout(output), status_pid_field(field))
}

pub fn status_json_pid(stdout: &str, field: &str) -> u32 {
    let value: serde_json::Value = serde_json::from_str(stdout).expect("status json");
    value[0][field].as_u64().expect("status pid field") as u32
}

fn status_pid_field(field: &str) -> &str {
    match field {
        "pid" => "runtime_pid",
        other => other,
    }
}

pub(super) fn parse_status_field(line: &str, key: &str) -> Option<u32> {
    line.split(&format!("{key}="))
        .nth(1)
        .and_then(|tail| tail.split_whitespace().next())
        .and_then(|value| (value != "-").then_some(value))
        .and_then(|value| value.parse().ok())
}
