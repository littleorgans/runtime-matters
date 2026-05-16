use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use rtm_core::{
    KillRequest, LaunchSpec, Lifecycle, LifecycleState, LostEvidence, RuntimeEvent, RuntimeExit,
    RuntimeSignal, ShimExit, ShimReady, SpawnRequest, TerminationEvidence,
};
use tokio::net::UnixListener;
use tokio::sync::{Mutex, broadcast, oneshot};
use uuid::Uuid;

use crate::{event_channel, handler, socket};

#[derive(Clone, Debug)]
pub struct DaemonConfig {
    pub socket_path: PathBuf,
    pub shim_path: PathBuf,
}

impl DaemonConfig {
    pub fn from_env() -> Result<Self> {
        let socket_path = socket::socket_path_from_env()?;
        let shim_path = match std::env::var_os("RTM_SHIM_PATH") {
            Some(path) => PathBuf::from(path),
            None => std::env::current_exe().context("failed to resolve current executable")?,
        };
        Ok(Self {
            socket_path,
            shim_path,
        })
    }
}

pub async fn run_daemon(config: DaemonConfig) -> Result<()> {
    rtm_launchers::warm_registry().context("failed to initialize launcher registry")?;
    socket::prepare_socket(&config.socket_path)?;
    let listener = UnixListener::bind(&config.socket_path)
        .with_context(|| format!("failed to bind {}", config.socket_path.display()))?;
    println!(
        "rtmd listening on {}",
        socket::display_socket_path(&config.socket_path)
    );

    let state = Arc::new(ServerState::new(config.clone()));
    let (shutdown_tx, mut shutdown_rx) = broadcast::channel(8);
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted.context("failed to accept daemon connection")?;
                let task_state = Arc::clone(&state);
                let task_shutdown = shutdown_tx.clone();
                tokio::spawn(async move {
                    if let Err(error) = handler::handle_connection(stream, task_state, task_shutdown).await {
                        tracing::warn!(%error, "daemon connection failed");
                    }
                });
            }
            _ = shutdown_rx.recv() => break,
            _ = tokio::signal::ctrl_c() => break,
            _ = terminate.recv() => break,
        }
    }

    state.terminate_shims().await;
    socket::remove_socket_file(&config.socket_path)?;
    Ok(())
}

pub(crate) struct ServerState {
    config: DaemonConfig,
    events: Mutex<Vec<RuntimeEvent>>,
    lifecycles: Mutex<HashMap<Uuid, Lifecycle>>,
    exit_watchers: Mutex<HashMap<Uuid, rtm_platform::kqueue::ProcessExitWatcher>>,
    pending_launches: Mutex<HashMap<Uuid, LaunchSpec>>,
    pending_ready: Mutex<HashMap<Uuid, oneshot::Sender<ShimReady>>>,
    terminated_events: Mutex<HashSet<Uuid>>,
}

impl ServerState {
    fn new(config: DaemonConfig) -> Self {
        Self {
            config,
            events: Mutex::new(Vec::new()),
            lifecycles: Mutex::new(HashMap::new()),
            exit_watchers: Mutex::new(HashMap::new()),
            pending_launches: Mutex::new(HashMap::new()),
            pending_ready: Mutex::new(HashMap::new()),
            terminated_events: Mutex::new(HashSet::new()),
        }
    }

    pub(crate) fn config(&self) -> &DaemonConfig {
        &self.config
    }

    pub(crate) async fn begin_spawn(
        &self,
        request: &SpawnRequest,
        launch: LaunchSpec,
    ) -> Result<oneshot::Receiver<ShimReady>> {
        if self
            .lifecycles
            .lock()
            .await
            .contains_key(&request.session_id)
        {
            bail!("session {} already exists", request.session_id);
        }

        self.lifecycles.lock().await.insert(
            request.session_id,
            Lifecycle::forking(request.session_id, request.runtime.clone()),
        );
        self.pending_launches
            .lock()
            .await
            .insert(request.session_id, launch);
        self.begin_ready_wait(request.session_id).await
    }

    async fn begin_ready_wait(&self, session_id: Uuid) -> Result<oneshot::Receiver<ShimReady>> {
        let (sender, receiver) = oneshot::channel();
        let previous = self.pending_ready.lock().await.insert(session_id, sender);
        if previous.is_some() {
            bail!("session {session_id} already has a pending shim");
        }
        Ok(receiver)
    }

    pub(crate) async fn cancel_spawn(&self, session_id: Uuid) {
        self.pending_launches.lock().await.remove(&session_id);
        self.pending_ready.lock().await.remove(&session_id);
        self.lifecycles.lock().await.remove(&session_id);
    }

    pub(crate) async fn take_launch_spec(&self, session_id: Uuid) -> Result<LaunchSpec> {
        self.pending_launches
            .lock()
            .await
            .remove(&session_id)
            .ok_or_else(|| anyhow!("no pending launch for session {session_id}"))
    }

    pub(crate) async fn complete_shim_ready(&self, ready: ShimReady) -> Result<()> {
        let sender = self
            .pending_ready
            .lock()
            .await
            .remove(&ready.session_id)
            .ok_or_else(|| anyhow!("no pending spawn for session {}", ready.session_id))?;
        sender
            .send(ready)
            .map_err(|ready| anyhow!("spawn waiter dropped for session {}", ready.session_id))
    }

    pub(crate) async fn record_running(
        self: &Arc<Self>,
        request: &SpawnRequest,
        ready: ShimReady,
    ) -> Result<(Lifecycle, RuntimeEvent)> {
        let runtime_pid = ready.runtime_pid;
        let lifecycle = {
            let mut lifecycles = self.lifecycles.lock().await;
            let lifecycle = lifecycles
                .get_mut(&request.session_id)
                .ok_or_else(|| anyhow!("no lifecycle for session {}", request.session_id))?;
            if lifecycle.runtime != request.runtime {
                bail!("runtime mismatch for session {}", request.session_id);
            }
            if !lifecycle.mark_running(ready) {
                bail!(
                    "session {} is not waiting for ShimReady",
                    request.session_id
                );
            }
            lifecycle.clone()
        };
        let event = event_channel::running_event(&lifecycle)?;

        self.start_exit_watcher(request.session_id, runtime_pid)
            .await?;
        self.events.lock().await.push(event.clone());
        Ok((lifecycle, event))
    }

    pub(crate) async fn kill_runtime(&self, request: KillRequest) -> Result<()> {
        let runtime_pid = self.runtime_pid(request.session_id).await?;
        rtm_platform::signal::send_signal(runtime_pid, request.signal)?;
        let deadline = Instant::now() + Duration::from_secs(request.grace_secs);

        while Instant::now() < deadline {
            if self.is_terminal(request.session_id).await
                || !rtm_platform::process::pid_alive(runtime_pid)
            {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }

        if rtm_platform::process::pid_alive(runtime_pid) && request.signal != RuntimeSignal::Kill {
            rtm_platform::signal::send_signal(runtime_pid, RuntimeSignal::Kill)?;
        }
        Ok(())
    }

    pub(crate) async fn record_shim_exit(&self, exit: ShimExit) -> Result<Option<RuntimeEvent>> {
        self.record_exited(exit.session_id, exit.exit, TerminationEvidence::ShimExit)
            .await
    }

    async fn record_watcher_exit(self: Arc<Self>, session_id: Uuid) -> Result<()> {
        tokio::time::sleep(Duration::from_millis(300)).await;
        if self.is_terminal(session_id).await {
            return Ok(());
        }

        let evidence = self.watcher_evidence(session_id).await?;
        match evidence {
            TerminationEvidence::Lost(lost) => {
                let _ = self.record_lost(session_id, lost).await?;
            }
            TerminationEvidence::KqueueExit => {
                let _ = self
                    .record_exited(session_id, RuntimeExit::new(None, None), evidence)
                    .await?;
            }
            TerminationEvidence::ShimExit => {}
        }
        Ok(())
    }

    pub(crate) async fn status(&self, session_id: Option<Uuid>) -> Vec<Lifecycle> {
        let mut rows: Vec<_> = self
            .lifecycles
            .lock()
            .await
            .values()
            .filter(|row| session_id.is_none_or(|id| row.session_id == id))
            .cloned()
            .collect();
        rows.sort_by_key(|row| row.session_id);
        rows
    }

    pub(crate) async fn events(&self) -> Vec<RuntimeEvent> {
        self.events.lock().await.clone()
    }

    pub(crate) async fn terminate_shims(&self) {
        let shim_pids: Vec<_> = self
            .lifecycles
            .lock()
            .await
            .values()
            .filter_map(|lifecycle| lifecycle.shim_pid)
            .collect();
        for pid in &shim_pids {
            if let Err(error) = rtm_platform::signal::send_signal(*pid, RuntimeSignal::Term) {
                tracing::warn!(%error, shim_pid = *pid, "failed to terminate shim");
            }
        }

        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline
            && shim_pids
                .iter()
                .any(|pid| rtm_platform::process::pid_alive(*pid))
        {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }

        for pid in shim_pids {
            if rtm_platform::process::pid_alive(pid)
                && let Err(error) = rtm_platform::signal::send_signal(pid, RuntimeSignal::Kill)
            {
                tracing::warn!(%error, shim_pid = pid, "failed to kill shim");
            }
        }
    }

    async fn start_exit_watcher(
        self: &Arc<Self>,
        session_id: Uuid,
        runtime_pid: u32,
    ) -> Result<()> {
        let (watcher, exit_rx) = rtm_platform::kqueue::watch_process_exit(runtime_pid)?;
        self.exit_watchers.lock().await.insert(session_id, watcher);
        let state = Arc::clone(self);
        tokio::spawn(async move {
            if exit_rx.await.is_ok()
                && let Err(error) = state.record_watcher_exit(session_id).await
            {
                tracing::warn!(%error, %session_id, "process exit watcher failed");
            }
        });
        Ok(())
    }

    async fn runtime_pid(&self, session_id: Uuid) -> Result<u32> {
        self.lifecycles
            .lock()
            .await
            .get(&session_id)
            .and_then(|lifecycle| lifecycle.runtime_pid)
            .ok_or_else(|| anyhow!("session {session_id} not found"))
    }

    async fn is_terminal(&self, session_id: Uuid) -> bool {
        self.lifecycles
            .lock()
            .await
            .get(&session_id)
            .is_some_and(|lifecycle| {
                matches!(
                    lifecycle.state,
                    LifecycleState::Exited(_) | LifecycleState::Lost(_)
                )
            })
    }

    async fn watcher_evidence(&self, session_id: Uuid) -> Result<TerminationEvidence> {
        let shim_pid = self
            .lifecycles
            .lock()
            .await
            .get(&session_id)
            .and_then(|lifecycle| lifecycle.shim_pid)
            .ok_or_else(|| anyhow!("session {session_id} missing shim pid"))?;
        if rtm_platform::process::pid_alive(shim_pid) {
            Ok(TerminationEvidence::KqueueExit)
        } else {
            Ok(TerminationEvidence::Lost(
                LostEvidence::ShimDiedBeforeReport,
            ))
        }
    }

    async fn record_exited(
        &self,
        session_id: Uuid,
        exit: RuntimeExit,
        evidence: TerminationEvidence,
    ) -> Result<Option<RuntimeEvent>> {
        let lifecycle = {
            let mut lifecycles = self.lifecycles.lock().await;
            let lifecycle = lifecycles
                .get_mut(&session_id)
                .ok_or_else(|| anyhow!("session {session_id} not found"))?;
            lifecycle.mark_exited(exit);
            lifecycle.clone()
        };
        self.finish_terminal(session_id, &lifecycle, evidence).await
    }

    async fn record_lost(
        &self,
        session_id: Uuid,
        evidence: LostEvidence,
    ) -> Result<Option<RuntimeEvent>> {
        let lifecycle = {
            let mut lifecycles = self.lifecycles.lock().await;
            let lifecycle = lifecycles
                .get_mut(&session_id)
                .ok_or_else(|| anyhow!("session {session_id} not found"))?;
            lifecycle.mark_lost(evidence);
            lifecycle.clone()
        };
        self.finish_terminal(session_id, &lifecycle, TerminationEvidence::Lost(evidence))
            .await
    }

    async fn finish_terminal(
        &self,
        session_id: Uuid,
        lifecycle: &Lifecycle,
        evidence: TerminationEvidence,
    ) -> Result<Option<RuntimeEvent>> {
        self.exit_watchers.lock().await.remove(&session_id);
        if !self.terminated_events.lock().await.insert(session_id) {
            return Ok(None);
        }
        let event = event_channel::terminated_event(lifecycle, evidence);
        self.events.lock().await.push(event.clone());
        Ok(Some(event))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn kill_unknown_session_returns_not_found() {
        let state = ServerState::new(DaemonConfig {
            socket_path: PathBuf::from("/tmp/rtm-test.sock"),
            shim_path: PathBuf::from("rtm"),
        });
        let request = KillRequest {
            session_id: Uuid::now_v7(),
            signal: RuntimeSignal::Term,
            grace_secs: 0,
        };

        let error = state.kill_runtime(request).await.expect_err("not found");
        assert!(error.to_string().contains("not found"), "{error}");
    }
}
