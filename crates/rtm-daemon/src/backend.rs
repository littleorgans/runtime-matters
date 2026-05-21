use anyhow::Result;
use lilo_rm_core::{IsolationPolicy, LaunchSpec, SpawnRequest};

use crate::{
    error::RuntimeFailure,
    server::DaemonConfig,
    shim_socket::{self, HeadlessLogPaths},
};

pub(crate) struct SpawnEvidence {
    pub(crate) log_paths: Option<HeadlessLogPaths>,
}

pub(crate) trait RuntimeBackend {
    async fn spawn(&self, request: &SpawnRequest, launch: &LaunchSpec) -> Result<SpawnEvidence>;
}

pub(crate) struct RuntimeBackends<'a> {
    host: HostRuntimeBackend<'a>,
}

impl<'a> RuntimeBackends<'a> {
    pub(crate) fn new(config: &'a DaemonConfig) -> Self {
        Self {
            host: HostRuntimeBackend { config },
        }
    }

    pub(crate) async fn spawn(
        &self,
        request: &SpawnRequest,
        launch: &LaunchSpec,
    ) -> Result<SpawnEvidence> {
        match &request.isolation {
            IsolationPolicy::Host => self.host.spawn(request, launch).await,
            IsolationPolicy::Docker(_) => Err(RuntimeFailure::unsupported_isolation_policy(
                request.isolation.to_string(),
            )),
        }
    }
}

struct HostRuntimeBackend<'a> {
    config: &'a DaemonConfig,
}

impl RuntimeBackend for HostRuntimeBackend<'_> {
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
        SpawnRequest, SpawnTarget,
    };
    use uuid::Uuid;

    use super::RuntimeBackends;
    use crate::server::DaemonConfig;

    #[tokio::test]
    async fn docker_policy_is_not_selected_for_host_backend() {
        let config = daemon_config();
        let backends = RuntimeBackends::new(&config);
        let mut request = spawn_request();
        request.isolation = IsolationPolicy::Docker(IsolationProfile {
            name: Some("locked".to_owned()),
        });

        let error = match backends.spawn(&request, &launch_spec()).await {
            Ok(_) => panic!("docker policy should stay unsupported until Docker backend lands"),
            Err(error) => error,
        };

        assert!(
            error
                .to_string()
                .contains("isolation policy docker:locked is not supported"),
            "{error:#}"
        );
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
            docker_preflight: Default::default(),
        }
    }

    fn spawn_request() -> SpawnRequest {
        SpawnRequest {
            session_id: Uuid::nil(),
            runtime: RuntimeKind::Claude,
            isolation: IsolationPolicy::Host,
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
