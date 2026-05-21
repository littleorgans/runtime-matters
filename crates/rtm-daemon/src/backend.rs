use anyhow::Result;
use lilo_rm_core::{IsolationPolicy, LaunchSpec, SpawnRequest};

use crate::{
    docker_runtime,
    server::DaemonConfig,
    shim_socket::{self, HeadlessLogPaths},
};

pub(crate) struct SpawnEvidence {
    pub(crate) log_paths: Option<HeadlessLogPaths>,
}

pub(crate) trait RuntimeBackend {
    fn prepare_launch(&self, request: &SpawnRequest, launch: LaunchSpec) -> Result<LaunchSpec>;

    async fn spawn(&self, request: &SpawnRequest, launch: &LaunchSpec) -> Result<SpawnEvidence>;
}

pub(crate) struct RuntimeBackends<'a> {
    host: HostRuntimeBackend<'a>,
    docker: DockerRuntimeBackend<'a>,
}

impl<'a> RuntimeBackends<'a> {
    pub(crate) fn new(config: &'a DaemonConfig) -> Self {
        Self {
            host: HostRuntimeBackend { config },
            docker: DockerRuntimeBackend { config },
        }
    }

    pub(crate) fn prepare_launch(
        &self,
        request: &SpawnRequest,
        launch: LaunchSpec,
    ) -> Result<LaunchSpec> {
        match &request.isolation {
            IsolationPolicy::Host => self.host.prepare_launch(request, launch),
            IsolationPolicy::Docker(_) => self.docker.prepare_launch(request, launch),
        }
    }

    pub(crate) async fn spawn(
        &self,
        request: &SpawnRequest,
        launch: &LaunchSpec,
    ) -> Result<SpawnEvidence> {
        match &request.isolation {
            IsolationPolicy::Host => self.host.spawn(request, launch).await,
            IsolationPolicy::Docker(_) => self.docker.spawn(request, launch).await,
        }
    }
}

struct HostRuntimeBackend<'a> {
    config: &'a DaemonConfig,
}

impl RuntimeBackend for HostRuntimeBackend<'_> {
    fn prepare_launch(&self, _request: &SpawnRequest, launch: LaunchSpec) -> Result<LaunchSpec> {
        Ok(launch)
    }

    async fn spawn(&self, request: &SpawnRequest, launch: &LaunchSpec) -> Result<SpawnEvidence> {
        let _ = launch;
        let log_paths = shim_socket::launch_shim(self.config, request).await?;
        Ok(SpawnEvidence { log_paths })
    }
}

struct DockerRuntimeBackend<'a> {
    config: &'a DaemonConfig,
}

impl RuntimeBackend for DockerRuntimeBackend<'_> {
    fn prepare_launch(&self, request: &SpawnRequest, launch: LaunchSpec) -> Result<LaunchSpec> {
        let IsolationPolicy::Docker(profile) = &request.isolation else {
            return Ok(launch);
        };
        docker_runtime::docker_run_launch(
            request.session_id,
            profile,
            self.config.docker_preflight.image_for(request)?,
            &launch,
            &request.target,
        )
    }

    async fn spawn(&self, request: &SpawnRequest, launch: &LaunchSpec) -> Result<SpawnEvidence> {
        let _ = launch;
        let log_paths = shim_socket::launch_shim(self.config, request).await?;
        Ok(SpawnEvidence { log_paths })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use lilo_rm_core::{
        HeadlessSpawnTarget, IsolationPolicy, IsolationProfile, LaunchEnv, LaunchSpec, RuntimeKind,
        SpawnRequest, SpawnTarget, TmuxAddress, TmuxSpawnTarget,
    };
    use uuid::Uuid;

    use super::RuntimeBackends;
    use crate::{docker_preflight::DockerPreflightConfig, server::DaemonConfig};

    #[test]
    fn docker_policy_wraps_launch_for_host_shim() {
        let config = daemon_config();
        let backends = RuntimeBackends::new(&config);
        let mut request = spawn_request();
        request.isolation = IsolationPolicy::Docker(IsolationProfile::default());
        request.image = Some("runtime-matters-agent:latest".to_owned());

        let launch = backends
            .prepare_launch(&request, launch_spec())
            .expect("prepare launch");

        assert!(launch.argv[0].ends_with("docker"));
        assert!(
            launch
                .argv
                .contains(&"runtime-matters-agent:latest".to_owned())
        );
    }

    #[test]
    fn docker_tmux_policy_uses_host_shim_attach_wrapper() {
        let config = daemon_config();
        let backends = RuntimeBackends::new(&config);
        let mut request = spawn_request();
        request.isolation = IsolationPolicy::Docker(IsolationProfile::default());
        request.target = SpawnTarget::Tmux(TmuxSpawnTarget {
            address: "rtm:0.1".parse::<TmuxAddress>().expect("tmux address"),
        });

        let launch = backends
            .prepare_launch(&request, launch_spec())
            .expect("prepare launch");

        assert_eq!(launch.argv[0], "/bin/sh");
        assert!(launch.argv[2].contains("'run'"));
        assert!(launch.argv[2].contains(" attach "));
    }

    fn daemon_config() -> DaemonConfig {
        DaemonConfig {
            endpoint: rtm_paths::RuntimeEndpoint::unix_socket("/tmp/rtm.sock"),
            shim_path: PathBuf::from("/tmp/rtm-shim"),
            log_root: PathBuf::from("/tmp/rtm/logs"),
            store: rtm_store::StoreConfig {
                db_path: PathBuf::from("/tmp/rtm.db"),
            },
            reconcile: Default::default(),
            docker_preflight: DockerPreflightConfig::new(
                "runtime-matters-agent:latest",
                false,
                false,
            ),
        }
    }

    fn spawn_request() -> SpawnRequest {
        SpawnRequest {
            session_id: Uuid::nil(),
            runtime: RuntimeKind::Claude,
            isolation: IsolationPolicy::Host,
            image: None,
            env: vec![],
            cwd: PathBuf::from("/tmp"),
            target: SpawnTarget::Headless(HeadlessSpawnTarget {}),
            force: false,
            shell_resume: None,
        }
    }

    fn launch_spec() -> LaunchSpec {
        LaunchSpec {
            argv: vec!["claude".to_owned()],
            env: vec![LaunchEnv::new("RTM_TEST", "1")],
            cwd: PathBuf::from("/tmp"),
            shell_resume: None,
        }
    }
}
