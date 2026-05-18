use anyhow::{Context, Result, bail};
use lilo_rm_core::{LaunchEnv, TmuxAddress};
use tokio::process::Command;

pub struct TmuxGateway;

impl TmuxGateway {
    pub async fn version() -> Result<Option<String>> {
        let Some(output) = tmux_output(["-V"]).await? else {
            return Ok(None);
        };
        if !output.status.success() {
            return Ok(None);
        }
        Ok(Some(stdout(output).trim().to_owned()))
    }

    pub async fn nudge(tmux_pane: &TmuxAddress, content: &str) -> Result<()> {
        if !Self::is_alive(tmux_pane).await? {
            bail!("tmux pane {tmux_pane} is not alive");
        }
        let target = tmux_pane.to_string();
        for trailing in build_nudge_send_keys_steps(content) {
            send_keys(&target, &trailing).await?;
        }
        Ok(())
    }

    pub async fn respawn_pane(
        tmux_pane: &TmuxAddress,
        argv: &[String],
        env: &[LaunchEnv],
    ) -> Result<()> {
        let args = build_respawn_pane_args(tmux_pane, argv, env)?;
        let output = tmux_output_owned(args)
            .await?
            .context("tmux is not installed")?;
        ensure_success(output, "tmux respawn-pane").map(|_| ())
    }

    pub async fn is_alive(tmux_pane: &TmuxAddress) -> Result<bool> {
        let has_session = tmux_output(["has-session", "-t", &tmux_pane.session]).await?;
        let Some(has_session) = has_session else {
            return Ok(false);
        };
        if !has_session.status.success() {
            return Ok(false);
        }

        let panes = tmux_output([
            "list-panes",
            "-s",
            "-t",
            &tmux_pane.session,
            "-F",
            "#S:#I.#P",
        ])
        .await?
        .context("tmux is not installed")?;
        ensure_success(panes, "tmux list-panes").map(|stdout| {
            stdout
                .lines()
                .any(|line| line.trim() == tmux_pane.to_string())
        })
    }
}

async fn tmux_output<const N: usize>(args: [&str; N]) -> Result<Option<std::process::Output>> {
    tmux_output_owned(args.into_iter().map(str::to_owned).collect()).await
}

async fn send_keys(target: &str, trailing: &[String]) -> Result<()> {
    let mut args = vec!["send-keys".to_owned(), "-t".to_owned(), target.to_owned()];
    args.extend(trailing.iter().cloned());
    let output = tmux_output_owned(args)
        .await?
        .context("tmux is not installed")?;
    ensure_success(output, "tmux send-keys").map(|_| ())
}

async fn tmux_output_owned(args: Vec<String>) -> Result<Option<std::process::Output>> {
    match Command::new("tmux").args(args).output().await {
        Ok(output) => Ok(Some(output)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).context("failed to run tmux"),
    }
}

fn ensure_success(output: std::process::Output, label: &'static str) -> Result<String> {
    if output.status.success() {
        return Ok(stdout(output));
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    bail!("{label} failed: {}", stderr.trim())
}

fn stdout(output: std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

/// Build the per-step argv trailers for the three send-keys invocations used
/// to deliver a nudge.
///
/// Mirrors nancy's pattern: send the literal payload first so bracketed-paste
/// mode does not eat shell metacharacters, then a hex CR (`0d`) to flush any
/// terminal paste buffer, then a real `Enter` to submit. Without the final
/// `Enter`, agents like Claude Code see the payload typed but never submitted.
fn build_nudge_send_keys_steps(content: &str) -> [Vec<String>; 3] {
    [
        vec!["-l".to_owned(), content.to_owned()],
        vec!["-H".to_owned(), "0d".to_owned()],
        vec!["Enter".to_owned()],
    ]
}

fn build_respawn_pane_args(
    tmux_pane: &TmuxAddress,
    argv: &[String],
    env: &[LaunchEnv],
) -> Result<Vec<String>> {
    if argv.is_empty() {
        bail!("tmux respawn-pane requires argv");
    }
    let mut args = vec![
        "respawn-pane".to_owned(),
        "-k".to_owned(),
        "-t".to_owned(),
        tmux_pane.to_string(),
    ];
    for entry in env {
        args.push("-e".to_owned());
        args.push(format!("{}={}", entry.key, entry.value));
    }
    args.push("--".to_owned());
    args.extend(argv.iter().cloned());
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane() -> TmuxAddress {
        "rtm:0.1".parse().expect("pane parse")
    }

    #[test]
    fn respawn_pane_args_only_carry_provided_env() {
        let env = vec![LaunchEnv::new("RTM_SOCKET_PATH", "/tmp/rtm.sock")];
        let argv = vec!["rtm".to_owned(), "__shim".to_owned()];
        let args = build_respawn_pane_args(&pane(), &argv, &env).expect("args");

        let mut e_flags = Vec::new();
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            if arg == "-e" {
                e_flags.push(iter.next().expect("paired -e value").clone());
            }
        }
        assert_eq!(e_flags, vec!["RTM_SOCKET_PATH=/tmp/rtm.sock".to_owned()]);
    }

    #[test]
    fn respawn_pane_args_never_leak_runtime_env() {
        // Regression guard: the daemon hands tmux only the bootstrap socket var.
        // Runtime env (secrets, PATH, etc.) must travel over the post-spawn UDS
        // ShimLaunch handoff, never via tmux's -e flag or argv.
        let env = vec![LaunchEnv::new("RTM_SOCKET_PATH", "/tmp/rtm.sock")];
        let argv = vec!["rtm".to_owned(), "__shim".to_owned()];
        let args = build_respawn_pane_args(&pane(), &argv, &env).expect("args");

        let e_values: Vec<&str> = args
            .windows(2)
            .filter(|pair| pair[0] == "-e")
            .map(|pair| pair[1].as_str())
            .collect();
        assert_eq!(e_values, vec!["RTM_SOCKET_PATH=/tmp/rtm.sock"]);

        for forbidden in [
            "HELIOY_PAT=",
            "ANTHROPIC_API_KEY=",
            "PATH=",
            "CLAUDE_CODE_SESSION_ID=",
        ] {
            assert!(
                !e_values.iter().any(|v| v.starts_with(forbidden)),
                "respawn-pane -e values leaked {forbidden}: {e_values:?}"
            );
        }
    }

    #[test]
    fn nudge_steps_are_literal_then_cr_then_enter() {
        let steps = build_nudge_send_keys_steps("hello world");
        assert_eq!(steps[0], vec!["-l".to_owned(), "hello world".to_owned()]);
        assert_eq!(steps[1], vec!["-H".to_owned(), "0d".to_owned()]);
        assert_eq!(steps[2], vec!["Enter".to_owned()]);
    }

    #[test]
    fn nudge_steps_preserve_special_chars_verbatim() {
        // -l means literal: payload bytes must be sent unmodified including
        // backticks, dollar signs, quotes. Submission is the separate Enter
        // step, not parsed from the payload.
        let steps = build_nudge_send_keys_steps("echo \"$PWD\" && ls -la");
        assert_eq!(steps[0][1], "echo \"$PWD\" && ls -la");
    }

    #[test]
    fn respawn_pane_args_reject_empty_argv() {
        let err =
            build_respawn_pane_args(&pane(), &[], &[]).expect_err("empty argv should be rejected");
        assert!(
            err.to_string().contains("requires argv"),
            "unexpected error: {err}"
        );
    }
}
