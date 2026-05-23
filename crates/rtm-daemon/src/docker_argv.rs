use std::path::Path;

use anyhow::Result;
use lilo_rm_core::{IsolationProfile, LaunchEnv, LaunchSpec, MountSpec, SpawnTarget};
use uuid::Uuid;

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
    mounts: &[MountSpec],
    target: &SpawnTarget,
    docker_command: &str,
) -> Result<LaunchSpec> {
    let command = launch.command()?;
    let tmux_target = matches!(target, SpawnTarget::Tmux(_));
    let mut run_argv = docker_run_argv(
        session_id,
        profile,
        image,
        launch,
        mounts,
        tmux_target,
        docker_command,
    );
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
    mounts: &[MountSpec],
    tmux_target: bool,
    docker_command: &str,
) -> Vec<String> {
    let cwd = path_arg(&launch.cwd);
    let mut argv = docker_run_base_argv(session_id, cwd, mounts, tmux_target, docker_command);
    if profile.name.as_deref() != Some("own-init") {
        argv.push("--init".to_owned());
    }
    append_env_args(&mut argv, &launch.env);
    argv.push(image.to_owned());
    argv
}

fn docker_run_base_argv(
    session_id: Uuid,
    cwd: String,
    mounts: &[MountSpec],
    tty: bool,
    docker_command: &str,
) -> Vec<String> {
    let cwd_mount = format!("type=bind,src={cwd},dst={cwd}");
    let mut argv = vec![
        docker_command.to_owned(),
        "run".to_owned(),
        "--rm".to_owned(),
        "--name".to_owned(),
        container_name(session_id),
        "--label".to_owned(),
        format!("{RTM_DOCKER_SESSION_LABEL}={session_id}"),
        "--mount".to_owned(),
        cwd_mount,
    ];
    append_mount_args(&mut argv, mounts);
    argv.extend(["--workdir".to_owned(), cwd]);
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

fn append_mount_args(argv: &mut Vec<String>, mounts: &[MountSpec]) {
    for mount in mounts {
        argv.push("--mount".to_owned());
        argv.push(bind_mount_arg(mount));
    }
}

fn bind_mount_arg(mount: &MountSpec) -> String {
    let mut arg = format!(
        "type=bind,source={},target={}",
        path_arg(&mount.source),
        path_arg(&mount.target)
    );
    if mount.read_only {
        arg.push_str(",readonly");
    }
    arg
}

fn path_arg(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use lilo_rm_core::{
        HeadlessSpawnTarget, IsolationProfile, LaunchEnv, LaunchSpec, MountSpec, SpawnTarget,
    };
    use uuid::Uuid;

    use super::{container_name, docker_run_launch};

    #[test]
    fn docker_run_launch_wraps_runtime_without_losing_launcher_env() {
        let session_id = Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        let launch = launch_spec(
            &["claude", "--print"],
            vec![LaunchEnv::new("CLAUDE_CODE", "1")],
        );

        let spec = docker_run_launch(
            session_id,
            &IsolationProfile::default(),
            "runtime-matters-agent:latest",
            &launch,
            &[],
            &SpawnTarget::Headless(HeadlessSpawnTarget {}),
            "docker",
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
        let launch = launch_spec(&["codex"], vec![LaunchEnv::new("CODEX", "1")]);

        let spec = docker_run_launch(
            session_id,
            &IsolationProfile {
                name: Some("own-init".to_owned()),
            },
            "runtime-matters-agent:latest",
            &launch,
            &[],
            &SpawnTarget::Headless(HeadlessSpawnTarget {}),
            "docker",
        )
        .expect("docker launch");

        assert!(!spec.argv.contains(&"--init".to_owned()));
    }

    #[test]
    fn docker_run_launch_uses_container_command_for_host_resolved_launcher() {
        let host_launcher = "/Users/alphab/.local/bin/claude";
        let launch = launch_spec(
            &[host_launcher, "--print"],
            vec![LaunchEnv::new("CLAUDE_CODE", "1")],
        );

        let spec = docker_run_launch(
            Uuid::nil(),
            &IsolationProfile::default(),
            "runtime-matters-agent:latest",
            &launch,
            &[],
            &SpawnTarget::Headless(HeadlessSpawnTarget {}),
            "docker",
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
        let launch = launch_spec(
            &["claude", "--dangerously-skip"],
            vec![LaunchEnv::new("CLAUDE_CODE", "1")],
        );

        let spec = docker_run_launch(
            session_id,
            &IsolationProfile::default(),
            "runtime-matters-agent:latest",
            &launch,
            &[],
            &"tmux:rtm:0.1".parse::<SpawnTarget>().expect("tmux target"),
            "docker",
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
        let launch = launch_spec(
            &["claude", "it's-safe"],
            vec![LaunchEnv::new("RTM_QUOTE", "it's-safe")],
        );

        let spec = docker_run_launch(
            Uuid::nil(),
            &IsolationProfile::default(),
            "runtime-matters-agent:latest",
            &launch,
            &[],
            &"tmux:rtm:0.1".parse::<SpawnTarget>().expect("tmux target"),
            "docker",
        )
        .expect("docker tmux launch");

        assert!(spec.argv[2].contains("'RTM_QUOTE=it'\\''s-safe'"));
        assert!(spec.argv[2].contains("'it'\\''s-safe'"));
    }

    #[test]
    fn docker_run_launch_emits_declared_mounts_in_order() {
        let launch = launch_spec(&["claude"], vec![]);
        let mounts = vec![
            MountSpec {
                source: "/canonical/host/claude".into(),
                target: "/home/agent/.claude".into(),
                read_only: true,
            },
            MountSpec {
                source: "/canonical/host/cache".into(),
                target: "/tmp/claude-cache".into(),
                read_only: false,
            },
        ];

        let spec = docker_run_launch(
            Uuid::nil(),
            &IsolationProfile::default(),
            "runtime-matters-agent:latest",
            &launch,
            &mounts,
            &SpawnTarget::Headless(HeadlessSpawnTarget {}),
            "docker",
        )
        .expect("docker launch");

        let cwd_mount_index = spec
            .argv
            .iter()
            .position(|arg| arg == "type=bind,src=/workspace/project,dst=/workspace/project")
            .expect("cwd mount");
        let image_index = spec
            .argv
            .iter()
            .position(|arg| arg == "runtime-matters-agent:latest")
            .expect("image");
        let declared_mounts = &spec.argv[cwd_mount_index + 1..cwd_mount_index + 5];
        assert!(cwd_mount_index < image_index);
        assert_eq!(
            declared_mounts,
            [
                "--mount",
                "type=bind,source=/canonical/host/claude,target=/home/agent/.claude,readonly",
                "--mount",
                "type=bind,source=/canonical/host/cache,target=/tmp/claude-cache",
            ]
        );
        assert!(cwd_mount_index + declared_mounts.len() < image_index);
        insta::assert_debug_snapshot!(spec.argv);
    }

    fn launch_spec(argv: &[&str], env: Vec<LaunchEnv>) -> LaunchSpec {
        LaunchSpec {
            argv: argv.iter().map(|arg| (*arg).to_owned()).collect(),
            env,
            cwd: PathBuf::from("/workspace/project"),
            shell_resume: None,
        }
    }
}
