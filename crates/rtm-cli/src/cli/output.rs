use std::io::Write;

use anyhow::Result;
use clap::{Args, ValueEnum};
use lilo_rm_core::CliOutput;
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

#[macro_export]
macro_rules! assert_cli_json_snapshot {
    ($output:expr, $redact:expr) => {{
        let mut value: serde_json::Value = serde_json::from_str(&$output).expect("cli json output");
        $redact(&mut value);
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
}
