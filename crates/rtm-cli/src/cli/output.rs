use std::io::Write;

use anyhow::Result;
use clap::{Args, ValueEnum};
use lilo_rm_core::CliOutput;
use serde_json::Value;
use serde_json::json;

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum OutputFormat {
    Json,
    Human,
}

#[derive(Debug, Args)]
pub struct OutputArgs {
    #[arg(long, value_enum, default_value_t = OutputFormat::Json)]
    pub format: OutputFormat,
}

#[derive(serde::Serialize)]
pub struct RtmError<'a> {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<&'a serde_json::Value>,
}

pub fn emit<T: CliOutput>(args: &OutputArgs, response: &T) -> Result<()> {
    match args.format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(response)?);
        }
        OutputFormat::Human => {
            let mut rendered = String::new();
            response.render_human(&mut rendered)?;
            print!("{rendered}");
        }
    }
    Ok(())
}

pub fn emit_error(format: OutputFormat, error: &anyhow::Error) -> Result<()> {
    match format {
        OutputFormat::Json => {
            let details = error_chain(error);
            let details = (!details.is_empty()).then(|| json!({ "causes": details }));
            let payload = RtmError {
                code: "runtime_error",
                message: error.to_string(),
                details: details.as_ref(),
            };
            writeln!(
                std::io::stderr(),
                "{}",
                serde_json::to_string_pretty(&payload)?
            )?;
        }
        OutputFormat::Human => {
            writeln!(std::io::stderr(), "Error: {error:?}")?;
        }
    }
    Ok(())
}

pub fn requested_format_from_env() -> OutputFormat {
    requested_format(std::env::args_os().skip(1))
}

fn requested_format(args: impl IntoIterator<Item = impl AsRef<std::ffi::OsStr>>) -> OutputFormat {
    let mut previous_was_format = false;
    for arg in args {
        let value = arg.as_ref().to_string_lossy();
        if previous_was_format {
            return parse_format(&value);
        }
        previous_was_format = value == "--format";
        if let Some(value) = value.strip_prefix("--format=") {
            return parse_format(value);
        }
    }
    OutputFormat::Json
}

fn parse_format(value: &str) -> OutputFormat {
    match value {
        "human" => OutputFormat::Human,
        _ => OutputFormat::Json,
    }
}

fn error_chain(error: &anyhow::Error) -> Vec<String> {
    error.chain().skip(1).map(ToString::to_string).collect()
}

const CLI_JSON_SNAPSHOT_REDACTIONS: &[(&str, &str)] = &[
    ("session_id", "[uuid]"),
    ("pid", "[pid]"),
    ("shim_pid", "[pid]"),
    ("runtime_pid", "[pid]"),
    ("started_at", "[timestamp]"),
    ("start_time", "[timestamp]"),
    ("applied_at", "[timestamp]"),
    ("last_probe_sweep", "[timestamp]"),
    ("uptime_ms", "[uptime]"),
    ("uptime_secs", "[uptime]"),
    ("socket", "[socket]"),
    ("socket_path", "[socket]"),
    ("log_dir", "[path]"),
    ("stdout_path", "[path]"),
    ("stderr_path", "[path]"),
    ("git_sha", "[git_sha]"),
    ("tmux_pane", "[tmux_pane]"),
    ("tmux", "[tmux]"),
    ("available", "[tmux]"),
    ("version", "[version]"),
    ("forking", "[count]"),
    ("running", "[count]"),
    ("exited", "[count]"),
    ("lost", "[count]"),
    ("kqueue_watchers", "[count]"),
    ("shim_sockets", "[count]"),
    ("command", "[command]"),
    ("error", "[launcher_error]"),
    ("message", "[message]"),
    ("cause", "[cause]"),
];

pub fn redact_cli_json_snapshot(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            for (field, replacement) in CLI_JSON_SNAPSHOT_REDACTIONS {
                if let Some(value) = fields.get_mut(*field)
                    && !value.is_object()
                    && !value.is_array()
                {
                    *value = json!(replacement);
                }
            }
            if let Some(Value::Array(causes)) = fields.get_mut("causes") {
                for cause in causes {
                    *cause = json!("[cause]");
                }
            }
            for value in fields.values_mut() {
                redact_cli_json_snapshot(value);
            }
        }
        Value::Array(values) => {
            for value in values {
                redact_cli_json_snapshot(value);
            }
        }
        _ => {}
    }
}

#[macro_export]
macro_rules! assert_cli_json_snapshot {
    ($output:expr) => {{
        let mut value: serde_json::Value = serde_json::from_str(&$output).expect("cli json output");
        $crate::cli::output::redact_cli_json_snapshot(&mut value);
        insta::assert_json_snapshot!(value);
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requested_format_defaults_to_json() {
        assert_eq!(requested_format(["status"]), OutputFormat::Json);
    }

    #[test]
    fn requested_format_accepts_human_forms() {
        assert_eq!(
            requested_format(["status", "--format", "human"]),
            OutputFormat::Human
        );
        assert_eq!(
            requested_format(["status", "--format=human"]),
            OutputFormat::Human
        );
    }

    #[test]
    fn cli_json_snapshot_redaction_covers_public_contract_fields() {
        let mut value = json!({
            "pid": 1,
            "started_at": "now",
            "uptime_ms": 7,
            "log_dir": "/tmp/rtm",
            "stdout_path": "/tmp/stdout",
            "git_sha": "abc",
            "applied_at": "later",
            "session_id": "uuid",
            "runtime_pid": 2,
            "start_time": "then",
            "tmux_pane": "%1",
            "socket_path": "/tmp/socket",
            "version": "0.1.0",
            "last_probe_sweep": "soon",
            "forking": 3,
            "available": true,
            "command": "claude",
            "error": "missing",
            "details": {
                "causes": ["nested"]
            },
            "nested_object_is_preserved": {
                "version": "redacted inside",
                "protocol_version": "0.3"
            }
        });

        redact_cli_json_snapshot(&mut value);

        assert_eq!(value["pid"], "[pid]");
        assert_eq!(value["started_at"], "[timestamp]");
        assert_eq!(value["uptime_ms"], "[uptime]");
        assert_eq!(value["log_dir"], "[path]");
        assert_eq!(value["stdout_path"], "[path]");
        assert_eq!(value["git_sha"], "[git_sha]");
        assert_eq!(value["applied_at"], "[timestamp]");
        assert_eq!(value["session_id"], "[uuid]");
        assert_eq!(value["runtime_pid"], "[pid]");
        assert_eq!(value["start_time"], "[timestamp]");
        assert_eq!(value["tmux_pane"], "[tmux_pane]");
        assert_eq!(value["socket_path"], "[socket]");
        assert_eq!(value["version"], "[version]");
        assert_eq!(value["last_probe_sweep"], "[timestamp]");
        assert_eq!(value["forking"], "[count]");
        assert_eq!(value["available"], "[tmux]");
        assert_eq!(value["command"], "[command]");
        assert_eq!(value["error"], "[launcher_error]");
        assert_eq!(value["details"]["causes"][0], "[cause]");
        assert_eq!(value["nested_object_is_preserved"]["version"], "[version]");
        assert_eq!(
            value["nested_object_is_preserved"]["protocol_version"],
            "0.3"
        );
    }
}
