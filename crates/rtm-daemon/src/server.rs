use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use rtm_core::{
    KillRequest, LaunchSpec, Lifecycle, LifecycleState, LostEvidence, RuntimeEvent, RuntimeExit,
    RuntimeSignal, ShimExit, ShimReady, SpawnRequest, TerminationEvidence,
};
use rtm_store::{LifecycleStore, StoreConfig};
use tokio::net::UnixListener;
use tokio::sync::{Mutex, broadcast, oneshot};
use uuid::Uuid;

use crate::{event_channel, handler, reconcile, socket};

#[derive(Clone, Debug)]
pub struct DaemonConfig {
    pub socket_path: PathBuf,
    pub shim_path: PathBuf,
    pub store: StoreConfig,
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
            store: StoreConfig::from_env()?,
        })
    }
}

pub async fn run_daemon(config: DaemonConfig) -> Result<()> {
    rtm_launchers::warm_registry().context("failed to initialize launcher registry")?;
    let store = LifecycleStore::open(config.store.clone()).await?;
    socket::prepare_socket(&config.socket_path)?;
    let listener = UnixListener::bind(&config.socket_path)
        .with_context(|| format!("failed to bind {}", config.socket_path.display()))?;
    println!(
        "rtmd listening on {}",
        socket::display_socket_path(&config.socket_path)
    );

    let state = Arc::new(ServerState::new(config.clone(), store));
    reconcile::reconcile_startup(Arc::clone(&state), &reconcile::SystemProcessProbe).await?;
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

    socket::remove_socket_file(&config.socket_path)?;
    Ok(())
}

pub(crate) struct ServerState {
    config: DaemonConfig,
    store: LifecycleStore,
    events: Mutex<Vec<RuntimeEvent>>,
    exit_watchers: Mutex<HashMap<Uuid, rtm_platform::kqueue::ProcessExitWatcher>>,
    pending_launches: Mutex<HashMap<Uuid, LaunchSpec>>,
    pending_ready: Mutex<HashMap<Uuid, oneshot::Sender<ShimReady>>>,
    terminated_events: Mutex<HashSet<Uuid>>,
}

impl ServerState {
    pub(crate) fn new(config: DaemonConfig, store: LifecycleStore) -> Self {
        Self {
            config,
            store,
            events: Mutex::new(Vec::new()),
            exit_watchers: Mutex::new(HashMap::new()),
            pending_launches: Mutex::new(HashMap::new()),
            pending_ready: Mutex::new(HashMap::new()),
            terminated_events: Mutex::new(HashSet::new()),
        }
    }

    pub(crate) fn config(&self) -> &DaemonConfig {
        &self.config
    }

    pub(crate) fn store(&self) -> &LifecycleStore {
        &self.store
    }

    pub(crate) async fn begin_spawn(
        &self,
        request: &SpawnRequest,
        launch: LaunchSpec,
    ) -> Result<oneshot::Receiver<ShimReady>> {
        if self.store.get(request.session_id).await?.is_some() {
            bail!("session {} already exists", request.session_id);
        }

        let lifecycle = Lifecycle::forking(request.session_id, request.runtime.clone());
        self.store.insert_forking(&lifecycle).await?;
        self.pending_launches
            .lock()
            .await
            .insert(request.session_id, launch);
        match self.begin_ready_wait(request.session_id).await {
            Ok(receiver) => Ok(receiver),
            Err(error) => {
                self.cancel_spawn(request.session_id).await;
                Err(error)
            }
        }
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
        if let Err(error) = self.store.delete(session_id).await {
            tracing::warn!(%error, %session_id, "failed to delete canceled lifecycle");
        }
    }

    pub(crate) async fn take_launch_spec(&self, session_id: Uuid) -> Result<LaunchSpec> {
        self.pending_launches
            .lock()
            .await
            .remove(&session_id)
            .ok_or_else(|| anyhow!("no pending launch for session {session_id}"))
    }

    pub(crate) async fn complete_shim_ready(self: &Arc<Self>, ready: ShimReady) -> Result<()> {
        let sender = self.pending_ready.lock().await.remove(&ready.session_id);
        if let Some(sender) = sender {
            return sender
                .send(ready)
                .map_err(|ready| anyhow!("spawn waiter dropped for session {}", ready.session_id));
        }
        self.record_reconnected_ready(ready).await.map(|_| ())
    }

    pub(crate) async fn record_running(
        self: &Arc<Self>,
        request: &SpawnRequest,
        ready: ShimReady,
    ) -> Result<(Lifecycle, RuntimeEvent)> {
        let runtime_pid = ready.runtime_pid;
        let mut lifecycle = self
            .store
            .get(request.session_id)
            .await?
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
        self.store.update_lifecycle(&lifecycle).await?;
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
        match self.store.list(session_id).await {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(%error, "failed to read lifecycle status");
                Vec::new()
            }
        }
    }

    pub(crate) async fn events(&self) -> Vec<RuntimeEvent> {
        self.events.lock().await.clone()
    }

    pub(crate) async fn start_exit_watcher(
        self: &Arc<Self>,
        session_id: Uuid,
        runtime_pid: u32,
    ) -> Result<()> {
        if self.exit_watchers.lock().await.contains_key(&session_id) {
            return Ok(());
        }
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
        self.store
            .get(session_id)
            .await?
            .and_then(|lifecycle| lifecycle.runtime_pid)
            .ok_or_else(|| anyhow!("session {session_id} not found"))
    }

    async fn is_terminal(&self, session_id: Uuid) -> bool {
        self.store
            .get(session_id)
            .await
            .ok()
            .flatten()
            .is_some_and(|lifecycle| {
                matches!(
                    lifecycle.state,
                    LifecycleState::Exited(_) | LifecycleState::Lost(_)
                )
            })
    }

    async fn watcher_evidence(&self, session_id: Uuid) -> Result<TerminationEvidence> {
        let shim_pid = self
            .store
            .get(session_id)
            .await?
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
        let mut lifecycle = self
            .store
            .get(session_id)
            .await?
            .ok_or_else(|| anyhow!("session {session_id} not found"))?;
        if !lifecycle.mark_exited(exit) {
            return Ok(None);
        }
        self.store.update_lifecycle(&lifecycle).await?;
        self.finish_terminal(session_id, &lifecycle, evidence).await
    }

    pub(crate) async fn record_lost(
        &self,
        session_id: Uuid,
        evidence: LostEvidence,
    ) -> Result<Option<RuntimeEvent>> {
        let mut lifecycle = self
            .store
            .get(session_id)
            .await?
            .ok_or_else(|| anyhow!("session {session_id} not found"))?;
        if !lifecycle.mark_lost(evidence) {
            return Ok(None);
        }
        self.store.update_lifecycle(&lifecycle).await?;
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
        let event = match evidence {
            TerminationEvidence::Lost(lost) => event_channel::lost_event(lifecycle, lost),
            TerminationEvidence::ShimExit | TerminationEvidence::KqueueExit => {
                event_channel::terminated_event(lifecycle, evidence)
            }
        };
        self.events.lock().await.push(event.clone());
        Ok(Some(event))
    }

    async fn record_reconnected_ready(
        self: &Arc<Self>,
        ready: ShimReady,
    ) -> Result<Option<RuntimeEvent>> {
        let runtime_pid = ready.runtime_pid;
        let mut lifecycle = self
            .store
            .get(ready.session_id)
            .await?
            .ok_or_else(|| anyhow!("session {} not found", ready.session_id))?;
        match lifecycle.state {
            LifecycleState::Forking => {
                lifecycle.mark_running(ready);
                self.store.update_lifecycle(&lifecycle).await?;
                let event = event_channel::running_event(&lifecycle)?;
                self.start_exit_watcher(lifecycle.session_id, runtime_pid)
                    .await?;
                self.events.lock().await.push(event.clone());
                Ok(Some(event))
            }
            LifecycleState::Running => {
                self.start_exit_watcher(lifecycle.session_id, runtime_pid)
                    .await?;
                Ok(None)
            }
            LifecycleState::Exited(_) | LifecycleState::Lost(_) => {
                bail!("session {} is already terminal", lifecycle.session_id)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn kill_unknown_session_returns_not_found() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let store_config = StoreConfig {
            db_path: temp.path().join("rtm.sqlite"),
        };
        let store = LifecycleStore::open(store_config.clone())
            .await
            .expect("store");
        let state = ServerState::new(
            DaemonConfig {
                socket_path: PathBuf::from("/tmp/rtm-test.sock"),
                shim_path: PathBuf::from("rtm"),
                store: store_config,
            },
            store,
        );
        let request = KillRequest {
            session_id: Uuid::now_v7(),
            signal: RuntimeSignal::Term,
            grace_secs: 0,
        };

        let error = state.kill_runtime(request).await.expect_err("not found");
        assert!(error.to_string().contains("not found"), "{error}");
    }
}
