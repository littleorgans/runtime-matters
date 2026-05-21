use std::path::Path;
use std::process::Command as StdCommand;

use anyhow::Result;
use lilo_rm_core::{
    IsolationProfile, KillOutcome, LaunchEnv, LaunchSpec, RuntimeSignal, SpawnTarget,
};
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
    target: &SpawnTarget,
) -> Result<LaunchSpec> {
    let command = launch.command()?;
    let tmux_target = matches!(target, SpawnTarget::Tmux(_));
    let mut run_argv = docker_run_argv(session_id, profile, image, launch, tmux_target);
    run_argv.push(container_command(command));
    run_argv.extend(launch.argv.iter().skip(1).cloned());

    let argv = match target {
        SpawnTarget::Headless(_) => run_argv,
        SpawnTarget::Tmux(_) => docker_tmux_attach_argv(run_argv),
    };

    Ok(LaunchSpec {
        argv,
        env: launch.env.clone(),
        cwd: launch.cwd.clone(),
        shell_resume: launch.shell_resume.clone(),
    })
}

fn docker_run_argv(
    session_id: Uuid,
    profile: &IsolationProfile,
    image: &str,
    launch: &LaunchSpec,
    tmux_target: bool,
) -> Vec<String> {
    let cwd = path_arg(&launch.cwd);
    let mut argv = docker_run_base_argv(session_id, cwd, tmux_target);
    if profile.name.as_deref() != Some("own-init") {
        argv.push("--init".to_owned());
    }
    append_env_args(&mut argv, &launch.env);
    argv.push(image.to_owned());
    argv
}

fn docker_run_base_argv(session_id: Uuid, cwd: String, tty: bool) -> Vec<String> {
    let mut argv = vec![
        docker_command(),
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
    if tty {
        argv.extend(["-d".to_owned(), "-i".to_owned(), "-t".to_owned()]);
    }
    argv
}

fn docker_tmux_attach_argv(run_argv: Vec<String>) -> Vec<String> {
    let docker = shell_quote(&run_argv[0]);
    vec![
        "/bin/sh".to_owned(),
        "-c".to_owned(),
        format!(
            "set -e; container_id=$({}); exec {docker} attach --detach-keys '' --sig-proxy=false \"$container_id\"",
            shell_command(&run_argv),
        ),
    ]
}

fn docker_command() -> String {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|dir| dir.join("docker"))
        .find(|path| is_executable(path))
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| "docker".to_owned())
}

fn container_command(command: &str) -> String {
    let path = Path::new(command);
    if path.is_absolute() {
        return path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| command.to_owned());
    }
    command.to_owned()
}

fn is_executable(path: &Path) -> bool {
    path.is_file()
}

fn shell_command(argv: &[String]) -> String {
    argv.iter()
        .map(|arg| shell_quote(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_owned();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
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

pub(crate) struct DockerCliRuntime;

impl DockerCliRuntime {
    pub(crate) async fn running(&self, session_id: Uuid) -> Result<bool> {
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

    use lilo_rm_core::{HeadlessSpawnTarget, IsolationProfile, LaunchEnv, LaunchSpec, SpawnTarget};
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
            &SpawnTarget::Headless(HeadlessSpawnTarget {}),
        )
        .expect("docker launch");

        assert!(spec.argv[0].ends_with("docker"));
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
            &SpawnTarget::Headless(HeadlessSpawnTarget {}),
        )
        .expect("docker launch");

        assert!(!spec.argv.contains(&"--init".to_owned()));
    }

    #[test]
    fn docker_run_launch_uses_container_command_for_host_resolved_launcher() {
        let host_launcher = "/Users/alphab/.local/bin/claude";
        let launch = LaunchSpec {
            argv: vec![host_launcher.to_owned(), "--print".to_owned()],
            env: vec![LaunchEnv::new("CLAUDE_CODE", "1")],
            cwd: PathBuf::from("/workspace/project"),
            shell_resume: None,
        };

        let spec = docker_run_launch(
            Uuid::nil(),
            &IsolationProfile::default(),
            "runtime-matters-agent:latest",
            &launch,
            &SpawnTarget::Headless(HeadlessSpawnTarget {}),
        )
        .expect("docker launch");

        let image_index = spec
            .argv
            .iter()
            .position(|arg| arg == "runtime-matters-agent:latest")
            .expect("image arg");
        assert!(
            !spec.argv[image_index + 1..]
                .iter()
                .any(|arg| arg.starts_with("/Users/alphab"))
        );
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
    fn tmux_launch_starts_detached_container_and_attaches_without_detach_keys() {
        let session_id = Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap();
        let launch = LaunchSpec {
            argv: vec!["claude".to_owned(), "--dangerously-skip".to_owned()],
            env: vec![LaunchEnv::new("CLAUDE_CODE", "1")],
            cwd: PathBuf::from("/workspace/project"),
            shell_resume: None,
        };

        let spec = docker_run_launch(
            session_id,
            &IsolationProfile::default(),
            "runtime-matters-agent:latest",
            &launch,
            &"tmux:rtm:0.1".parse::<SpawnTarget>().expect("tmux target"),
        )
        .expect("docker tmux launch");

        assert_eq!(spec.argv[..2], ["/bin/sh".to_owned(), "-c".to_owned()]);
        let script = &spec.argv[2];
        assert!(script.contains("'run'"));
        assert!(script.contains("'-d' '-i' '-t'"));
        assert!(script.contains("'--rm'"));
        assert!(script.contains("'--name' 'rtm-22222222-2222-2222-2222-222222222222'"));
        assert!(script.contains("'runtime-matters-agent:latest' 'claude' '--dangerously-skip'"));
        assert!(script.contains(" attach --detach-keys '' --sig-proxy=false"));
    }

    #[test]
    fn shell_quote_preserves_single_quotes_in_docker_args() {
        let launch = LaunchSpec {
            argv: vec!["claude".to_owned(), "it's-safe".to_owned()],
            env: vec![LaunchEnv::new("RTM_QUOTE", "it's-safe")],
            cwd: PathBuf::from("/workspace/project"),
            shell_resume: None,
        };

        let spec = docker_run_launch(
            Uuid::nil(),
            &IsolationProfile::default(),
            "runtime-matters-agent:latest",
            &launch,
            &"tmux:rtm:0.1".parse::<SpawnTarget>().expect("tmux target"),
        )
        .expect("docker tmux launch");

        assert!(spec.argv[2].contains("'RTM_QUOTE=it'\\''s-safe'"));
        assert!(spec.argv[2].contains("'it'\\''s-safe'"));
    }
}
