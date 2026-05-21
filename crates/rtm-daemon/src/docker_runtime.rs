use std::path::Path;
use std::process::Command as StdCommand;

use anyhow::Result;
use lilo_rm_core::{IsolationProfile, KillOutcome, LaunchEnv, LaunchSpec, RuntimeSignal};
use tokio::process::Command;
use uuid::Uuid;

use crate::error::RuntimeFailure;

const RTM_DOCKER_CONTAINER_PREFIX: &str = "rtm";
const RTM_DOCKER_SESSION_LABEL: &str = "io.helioy.runtime-matters.session";

pub(crate) fn container_name(session_id: Uuid) -> String {
    format!("{RTM_DOCKER_CONTAINER_PREFIX}-{session_id}")
}

pub(crate) fn docker_run_launch(
    session_id: Uuid,
    profile: &IsolationProfile,
    image: &str,
    launch: &LaunchSpec,
) -> Result<LaunchSpec> {
    let command = launch.command()?;
    let cwd = path_arg(&launch.cwd);
    let mut argv = vec![
        "docker".to_owned(),
        "run".to_owned(),
        "--rm".to_owned(),
        "--name".to_owned(),
        container_name(session_id),
        "--label".to_owned(),
        format!("{RTM_DOCKER_SESSION_LABEL}={session_id}"),
        "--mount".to_owned(),
        format!("type=bind,src={cwd},dst={cwd}"),
        "--workdir".to_owned(),
        cwd,
    ];
    if profile.name.as_deref() != Some("own-init") {
        argv.push("--init".to_owned());
    }
    append_env_args(&mut argv, &launch.env);
    argv.push(image.to_owned());
    argv.push(command.to_owned());
    argv.extend(launch.argv.iter().skip(1).cloned());

    Ok(LaunchSpec {
        argv,
        env: launch.env.clone(),
        cwd: launch.cwd.clone(),
        shell_resume: launch.shell_resume.clone(),
    })
}

fn append_env_args(argv: &mut Vec<String>, env: &[LaunchEnv]) {
    for entry in env {
        argv.push("--env".to_owned());
        argv.push(format!("{}={}", entry.key, entry.value));
    }
}

fn path_arg(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

pub(crate) trait DockerContainerLiveness {
    async fn running(&self, session_id: Uuid) -> Result<bool>;
}

pub(crate) struct DockerCliRuntime;

impl DockerContainerLiveness for DockerCliRuntime {
    async fn running(&self, session_id: Uuid) -> Result<bool> {
        let output = Command::new("docker")
            .arg("container")
            .arg("inspect")
            .arg(container_name(session_id))
            .arg("--format")
            .arg("{{.State.Running}}")
            .output()
            .await
            .map_err(|error| RuntimeFailure::docker_unavailable(error.to_string()))?;

        if !output.status.success() {
            return Ok(false);
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim() == "true")
    }
}

pub(crate) fn container_running_blocking(session_id: Uuid) -> Result<bool> {
    let output = StdCommand::new("docker")
        .arg("container")
        .arg("inspect")
        .arg(container_name(session_id))
        .arg("--format")
        .arg("{{.State.Running}}")
        .output()
        .map_err(|error| RuntimeFailure::docker_unavailable(error.to_string()))?;

    if !output.status.success() {
        return Ok(false);
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim() == "true")
}

pub(crate) async fn kill_container(session_id: Uuid, signal: RuntimeSignal) -> Result<KillOutcome> {
    let mut command = Command::new("docker");
    command.arg("kill");
    command
        .arg("--signal")
        .arg(signal_number_arg(signal))
        .arg(container_name(session_id));

    let output = command
        .output()
        .await
        .map_err(|error| RuntimeFailure::docker_unavailable(error.to_string()))?;

    if output.status.success() {
        return Ok(KillOutcome::Signalled);
    }
    if command_stderr(&output.stderr).contains("No such container") {
        return Ok(KillOutcome::AlreadyExited);
    }
    Err(RuntimeFailure::docker_unavailable(command_stderr(
        &output.stderr,
    )))
}

fn signal_number_arg(signal: RuntimeSignal) -> String {
    rtm_platform::signal::signal_number(signal).to_string()
}

fn command_stderr(stderr: &[u8]) -> String {
    let message = String::from_utf8_lossy(stderr).trim().to_owned();
    if message.is_empty() {
        "docker command failed without stderr".to_owned()
    } else {
        message
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use lilo_rm_core::{IsolationProfile, LaunchEnv, LaunchSpec};
    use uuid::Uuid;

    use super::{container_name, docker_run_launch};

    #[test]
    fn docker_run_launch_wraps_runtime_without_losing_launcher_env() {
        let session_id = Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        let launch = LaunchSpec {
            argv: vec!["claude".to_owned(), "--print".to_owned()],
            env: vec![LaunchEnv::new("CLAUDE_CODE", "1")],
            cwd: PathBuf::from("/workspace/project"),
            shell_resume: None,
        };

        let spec = docker_run_launch(
            session_id,
            &IsolationProfile::default(),
            "runtime-matters-agent:latest",
            &launch,
        )
        .expect("docker launch");

        assert_eq!(spec.argv[0], "docker");
        assert!(spec.argv.contains(&"--init".to_owned()));
        assert!(spec.argv.contains(&container_name(session_id)));
        assert!(spec.argv.contains(&"CLAUDE_CODE=1".to_owned()));
        assert_eq!(
            spec.argv[spec.argv.len() - 3..],
            [
                "runtime-matters-agent:latest".to_owned(),
                "claude".to_owned(),
                "--print".to_owned(),
            ]
        );
    }

    #[test]
    fn own_init_profile_does_not_add_docker_init() {
        let session_id = Uuid::nil();
        let launch = LaunchSpec {
            argv: vec!["codex".to_owned()],
            env: vec![LaunchEnv::new("CODEX", "1")],
            cwd: PathBuf::from("/workspace/project"),
            shell_resume: None,
        };

        let spec = docker_run_launch(
            session_id,
            &IsolationProfile {
                name: Some("own-init".to_owned()),
            },
            "runtime-matters-agent:latest",
            &launch,
        )
        .expect("docker launch");

        assert!(!spec.argv.contains(&"--init".to_owned()));
    }
}
