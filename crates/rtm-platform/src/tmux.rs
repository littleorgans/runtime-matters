use anyhow::{Context, Result, bail};
use rtm_core::{LaunchEnv, TmuxAddress};
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
        let output = tmux_output_owned(vec![
            "send-keys".to_owned(),
            "-t".to_owned(),
            tmux_pane.to_string(),
            "-l".to_owned(),
            content.to_owned(),
        ])
        .await?
        .context("tmux is not installed")?;
        ensure_success(output, "tmux send-keys").map(|_| ())
    }

    pub async fn respawn_pane(
        tmux_pane: &TmuxAddress,
        argv: &[String],
        env: &[LaunchEnv],
    ) -> Result<()> {
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
