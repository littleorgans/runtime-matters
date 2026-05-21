use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use lilo_rm_core::{
    CaptureError, CaptureRequest, CaptureResponse, EventsRequest, KillByPidRequest,
    KillByPidResponse, KillOutcome, KillRequest, LaunchSpec, Lifecycle, LifecycleLogAvailability,
    LifecycleState, LogAvailability, LogsUnavailableReason, LostEvidence, NudgeFailureReason,
    NudgeOutcome, NudgeRequest, NudgeResponse, RuntimeEvent, RuntimeExit, RuntimeSignal, ShimExit,
    ShimReady, SpawnRequest, StatusFilter, TerminationEvidence, ValidateTargetOutcome,
    ValidateTargetRequest, ValidateTargetResponse, WatcherCounts,
};
use rtm_paths::{RuntimeEndpoint, RuntimePathEnv};
use rtm_platform::process_exit::{ProcessExitWatcher, watch_process_exit};
use rtm_store::{LifecycleStore, StoreConfig};
use tokio::net::UnixListener;
use tokio::sync::{Mutex, broadcast, oneshot};
use uuid::Uuid;

use crate::{
    docker_preflight::DockerPreflightConfig,
    error::RuntimeFailure,
    event_channel,
    event_log::{CursorExpired, EventBatch, EventLog},
    handler, reconcile, runtime_kill, socket,
};

#[derive(Clone, Debug)]
pub struct DaemonConfig {
    pub endpoint: RuntimeEndpoint,
    pub shim_path: PathBuf,
    pub log_root: PathBuf,
    pub store: StoreConfig,
    pub reconcile: reconcile::ReconcileConfig,
    pub docker_preflight: DockerPreflightConfig,
}

impl DaemonConfig {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            endpoint: socket::runtime_endpoint_from_env()?,
            shim_path: rtm_paths::shim_path_from_env()?,
            log_root: rtm_paths::log_root_from_env()?,
            store: StoreConfig::from_env()?,
            reconcile: reconcile::ReconcileConfig::from_env()?,
            docker_preflight: DockerPreflightConfig::from_env(),
        })
    }

    pub fn socket_path(&self) -> Result<&std::path::Path> {
        Ok(self.endpoint.unix_socket_path()?)
    }

    pub fn session_log_dir(&self, session_id: Uuid) -> PathBuf {
        self.log_root.join(session_id.to_string())
    }

    pub fn session_log_paths(&self, session_id: Uuid) -> crate::shim_socket::HeadlessLogPaths {
        let log_dir = self.session_log_dir(session_id);
        crate::shim_socket::HeadlessLogPaths {
            stdout_path: log_dir.join("stdout.log"),
            stderr_path: log_dir.join("stderr.log"),
            log_dir,
        }
    }

    pub fn data_dir(&self) -> PathBuf {
        self.store
            .db_path
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| self.log_root.clone())
    }
}

pub async fn run_daemon(config: DaemonConfig) -> Result<()> {
    rtm_launchers::warm_registry().context("failed to initialize launcher registry")?;
    let store = LifecycleStore::open(config.store.clone()).await?;
    let socket_path = config.socket_path()?;
    socket::prepare_socket(socket_path)?;
    let listener = UnixListener::bind(socket_path)
        .with_context(|| format!("failed to bind {}", socket_path.display()))?;
    println!(
        "rtmd listening on {}",
        config
            .endpoint
            .display_label(&RuntimePathEnv::from_process())
    );

    let state = Arc::new(ServerState::new(config.clone(), store)?);
    reconcile::reconcile_startup(Arc::clone(&state), &reconcile::SystemProcessProbe).await?;
    let (shutdown_tx, mut shutdown_rx) = broadcast::channel(8);
    let reconcile_task = tokio::spawn(reconcile::run_periodic(
        Arc::clone(&state),
        reconcile::SystemProcessProbe,
        shutdown_tx.subscribe(),
        config.reconcile,
    ));
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

    socket::remove_socket_file(config.socket_path()?)?;
    let _ = shutdown_tx.send(());
    if let Err(error) = reconcile_task.await {
        tracing::warn!(%error, "periodic reconciliation task failed");
    }
    Ok(())
}

pub(crate) struct ServerState {
    config: DaemonConfig,
    store: LifecycleStore,
    started_instant: Instant,
    event_log: EventLog,
    exit_watchers: Mutex<HashMap<Uuid, ProcessExitWatcher>>,
    pending_launches: Mutex<HashMap<Uuid, LaunchSpec>>,
    pending_ready: Mutex<HashMap<Uuid, oneshot::Sender<ShimReady>>>,
    terminated_events: Mutex<HashSet<Uuid>>,
}

impl ServerState {
    pub(crate) fn new(config: DaemonConfig, store: LifecycleStore) -> Result<Self> {
        Ok(Self {
            event_log: EventLog::open(config.data_dir())?,
            config,
            store,
            started_instant: Instant::now(),
            exit_watchers: Mutex::new(HashMap::new()),
            pending_launches: Mutex::new(HashMap::new()),
            pending_ready: Mutex::new(HashMap::new()),
            terminated_events: Mutex::new(HashSet::new()),
        })
    }

    pub(crate) fn config(&self) -> &DaemonConfig {
        &self.config
    }

    pub(crate) fn store(&self) -> &LifecycleStore {
        &self.store
    }

    pub(crate) fn uptime_secs(&self) -> u64 {
        self.started_instant.elapsed().as_secs()
    }

    pub(crate) async fn begin_spawn(
        &self,
        request: &SpawnRequest,
        launch: LaunchSpec,
    ) -> Result<oneshot::Receiver<ShimReady>> {
        if self.store.get(request.session_id).await?.is_some() {
            return Err(RuntimeFailure::session_already_exists(request.session_id));
        }
        self.validate_spawn_target(request).await?;

        let mut lifecycle = Lifecycle::forking(request.session_id, request.runtime.clone());
        lifecycle.isolation = request.isolation.clone();
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

    async fn validate_spawn_target(&self, request: &SpawnRequest) -> Result<()> {
        match self.validate_target(&request.target).await?.outcome {
            ValidateTargetOutcome::Valid => Ok(()),
            ValidateTargetOutcome::TmuxPaneDead { address } => {
                Err(RuntimeFailure::tmux_pane_dead(address))
            }
            ValidateTargetOutcome::InvalidTarget { message } => {
                Err(RuntimeFailure::protocol_mismatch(message))
            }
            ValidateTargetOutcome::UnsupportedTarget { target } => Err(
                RuntimeFailure::protocol_mismatch(format!("unsupported target {target}")),
            ),
        }
    }

    pub(crate) async fn validate_target_request(
        &self,
        request: ValidateTargetRequest,
    ) -> Result<ValidateTargetResponse> {
        let target = match request.target.parse() {
            Ok(target) => target,
            Err(error) => return Ok(ValidateTargetResponse::from_target_parse_error(error)),
        };
        self.validate_target(&target).await
    }

    async fn validate_target(
        &self,
        target: &lilo_rm_core::SpawnTarget,
    ) -> Result<ValidateTargetResponse> {
        if let Some(address) = target.tmux_address()
            && !rtm_platform::tmux::TmuxGateway::is_alive(address).await?
        {
            return Ok(ValidateTargetResponse::tmux_pane_dead(address.clone()));
        }
        Ok(ValidateTargetResponse::valid())
    }

    async fn begin_ready_wait(&self, session_id: Uuid) -> Result<oneshot::Receiver<ShimReady>> {
        let (sender, receiver) = oneshot::channel();
        let previous = self.pending_ready.lock().await.insert(session_id, sender);
        if previous.is_some() {
            return Err(RuntimeFailure::protocol_mismatch(format!(
                "session {session_id} already has a pending shim"
            )));
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
            .ok_or_else(|| {
                RuntimeFailure::protocol_mismatch(format!(
                    "no pending launch for session {session_id}"
                ))
            })
    }

    pub(crate) async fn complete_shim_ready(self: &Arc<Self>, ready: ShimReady) -> Result<()> {
        let sender = self.pending_ready.lock().await.remove(&ready.session_id);
        if let Some(sender) = sender {
            return sender.send(ready).map_err(|ready| {
                RuntimeFailure::protocol_mismatch(format!(
                    "spawn waiter dropped for session {}",
                    ready.session_id
                ))
            });
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
            .ok_or_else(|| RuntimeFailure::session_not_found(request.session_id))?;
        if lifecycle.runtime != request.runtime {
            return Err(RuntimeFailure::protocol_mismatch(format!(
                "runtime mismatch for session {}",
                request.session_id
            )));
        }
        if !lifecycle.mark_running(ready) {
            return Err(RuntimeFailure::protocol_mismatch(format!(
                "session {} is not waiting for ShimReady",
                request.session_id
            )));
        }
        lifecycle.tmux_pane = request.target.tmux_address().cloned();
        self.populate_log_availability(&mut lifecycle).await;
        self.store.update_lifecycle(&lifecycle).await?;
        let event = event_channel::running_event(&lifecycle)?;

        self.start_exit_watcher(request.session_id, runtime_pid)
            .await?;
        let event = self.append_event(event).await?;
        Ok((lifecycle, event))
    }

    pub(crate) async fn kill_runtime(&self, request: KillRequest) -> Result<KillOutcome> {
        runtime_kill::kill_runtime(self, request).await
    }

    pub(crate) async fn kill_pid(&self, request: KillByPidRequest) -> Result<KillByPidResponse> {
        let outcome = rtm_platform::signal::send_raw_signal_for_kill(request.pid, request.signal)?;
        if matches!(outcome, KillOutcome::AlreadyExited) {
            return Ok(KillByPidResponse {
                pid: request.pid,
                signal: request.signal,
                killed_after_grace: false,
                outcome,
            });
        }
        let deadline = Instant::now() + Duration::from_secs(request.grace_secs);

        while Instant::now() < deadline {
            if !rtm_platform::process::pid_alive(request.pid) {
                return Ok(KillByPidResponse {
                    pid: request.pid,
                    signal: request.signal,
                    killed_after_grace: false,
                    outcome,
                });
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }

        let mut killed_after_grace = false;
        let kill_signal = rtm_platform::signal::signal_number(RuntimeSignal::Kill);
        if rtm_platform::process::pid_alive(request.pid) && request.signal != kill_signal {
            rtm_platform::signal::send_raw_signal(request.pid, kill_signal)?;
            killed_after_grace = true;
        }
        Ok(KillByPidResponse {
            pid: request.pid,
            signal: request.signal,
            killed_after_grace,
            outcome,
        })
    }

    pub(crate) async fn nudge_runtime(&self, request: NudgeRequest) -> Result<NudgeResponse> {
        let lifecycle = self
            .store
            .get(request.session_id)
            .await?
            .ok_or_else(|| RuntimeFailure::session_not_found(request.session_id))?;
        let Some(tmux_pane) = lifecycle.tmux_pane.as_ref() else {
            return Ok(NudgeResponse {
                delivered: false,
                outcome: NudgeOutcome::Unsupported(NudgeFailureReason::HeadlessLifecycle),
            });
        };
        if self.is_terminal(request.session_id).await {
            return Ok(NudgeResponse {
                delivered: false,
                outcome: NudgeOutcome::Failed(NudgeFailureReason::SessionEnded),
            });
        }

        if !rtm_platform::tmux::TmuxGateway::nudge(tmux_pane, &request.content).await? {
            return Ok(NudgeResponse {
                delivered: false,
                outcome: NudgeOutcome::Failed(NudgeFailureReason::TmuxPaneDead),
            });
        }

        Ok(NudgeResponse {
            delivered: true,
            outcome: NudgeOutcome::Delivered,
        })
    }

    pub(crate) async fn capture_pane(&self, request: CaptureRequest) -> Result<CaptureResponse> {
        let Some(lifecycle) = self.store.get(request.session_id).await? else {
            return Ok(CaptureResponse::Failed(CaptureError::SessionMissing));
        };
        let Some(tmux_pane) = lifecycle.tmux_pane.as_ref() else {
            return Ok(CaptureResponse::Failed(CaptureError::NotATmuxTarget));
        };
        if !rtm_platform::tmux::TmuxGateway::is_alive(tmux_pane).await? {
            return Ok(CaptureResponse::Failed(CaptureError::PaneUnavailable));
        }
        let scrollback_lines = request.scrollback_lines.unwrap_or(1000);
        Ok(
            match rtm_platform::tmux::TmuxGateway::capture_pane(tmux_pane, scrollback_lines).await {
                Ok(snapshot) => CaptureResponse::Captured(snapshot),
                Err(error) => CaptureResponse::Failed(error),
            },
        )
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
            TerminationEvidence::ProcessExit => {
                let _ = self
                    .record_exited(session_id, RuntimeExit::new(None, None), evidence)
                    .await?;
            }
            TerminationEvidence::ShimExit => {}
            _ => bail!(
                "process exit watcher for session {session_id} received unsupported termination evidence variant: {evidence:?}"
            ),
        }
        Ok(())
    }

    pub(crate) async fn status(&self, filter: StatusFilter) -> Vec<Lifecycle> {
        match self.store.list(&filter).await {
            Ok(mut rows) => {
                self.populate_log_availability_for(&mut rows).await;
                rows
            }
            Err(error) => {
                tracing::warn!(%error, "failed to read lifecycle status");
                Vec::new()
            }
        }
    }

    pub(crate) async fn log_availability_statuses(&self) -> Vec<LifecycleLogAvailability> {
        self.status(StatusFilter::empty())
            .await
            .into_iter()
            .filter_map(|lifecycle| {
                lifecycle
                    .log_availability
                    .map(|log_availability| LifecycleLogAvailability {
                        session_id: lifecycle.session_id,
                        log_availability,
                    })
            })
            .collect()
    }

    async fn populate_log_availability_for(&self, lifecycles: &mut [Lifecycle]) {
        for lifecycle in lifecycles {
            self.populate_log_availability(lifecycle).await;
        }
    }

    async fn populate_log_availability(&self, lifecycle: &mut Lifecycle) {
        lifecycle.log_availability = Some(match lifecycle.tmux_pane.as_ref() {
            Some(address) => match rtm_platform::tmux::TmuxGateway::is_alive(address).await {
                Ok(true) => LogAvailability::TmuxPaneSnapshot,
                Ok(false) | Err(_) => LogAvailability::Unavailable {
                    reason: LogsUnavailableReason::PaneUnavailable,
                },
            },
            None => {
                let paths = self.config.session_log_paths(lifecycle.session_id);
                LogAvailability::Headless {
                    stdout_path: paths.stdout_path,
                    stderr_path: paths.stderr_path,
                }
            }
        });
    }

    pub(crate) async fn events(
        &self,
        request: EventsRequest,
    ) -> std::result::Result<EventBatch, CursorExpired> {
        self.event_log
            .events_since_or_wait(request.since, request.wait_ms)
            .await
    }

    pub(crate) async fn watcher_counts(&self) -> WatcherCounts {
        WatcherCounts {
            process_exit_watchers: self.exit_watchers.lock().await.len(),
            shim_sockets: self.pending_ready.lock().await.len(),
            event_waiters: self.event_log.waiter_count().await,
        }
    }

    pub(crate) async fn start_exit_watcher(
        self: &Arc<Self>,
        session_id: Uuid,
        runtime_pid: u32,
    ) -> Result<()> {
        if self.exit_watchers.lock().await.contains_key(&session_id) {
            return Ok(());
        }
        let (watcher, exit_rx) = watch_process_exit(runtime_pid)?;
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

    pub(crate) async fn is_terminal(&self, session_id: Uuid) -> bool {
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
            Ok(TerminationEvidence::ProcessExit)
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
            .ok_or_else(|| RuntimeFailure::session_not_found(session_id))?;
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
            .ok_or_else(|| RuntimeFailure::session_not_found(session_id))?;
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
            TerminationEvidence::ShimExit | TerminationEvidence::ProcessExit => {
                event_channel::terminated_event(lifecycle, evidence)
            }
            _ => bail!(
                "recording terminal event for session {session_id} received unsupported termination evidence variant: {evidence:?}"
            ),
        };
        Ok(Some(self.append_event(event).await?))
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
            .ok_or_else(|| RuntimeFailure::session_not_found(ready.session_id))?;
        match lifecycle.state {
            LifecycleState::Forking => {
                lifecycle.mark_running(ready);
                self.store.update_lifecycle(&lifecycle).await?;
                let event = event_channel::running_event(&lifecycle)?;
                self.start_exit_watcher(lifecycle.session_id, runtime_pid)
                    .await?;
                Ok(Some(self.append_event(event).await?))
            }
            LifecycleState::Running => {
                self.start_exit_watcher(lifecycle.session_id, runtime_pid)
                    .await?;
                Ok(None)
            }
            LifecycleState::Exited(_) | LifecycleState::Lost(_) => {
                bail!("session {} is already terminal", lifecycle.session_id)
            }
            _ => bail!(
                "reconnecting ShimReady for session {} saw unsupported lifecycle state variant: {:?}",
                lifecycle.session_id,
                lifecycle.state
            ),
        }
    }

    async fn append_event(&self, event: RuntimeEvent) -> Result<RuntimeEvent> {
        self.event_log.append(event).await
    }
}

#[cfg(test)]
mod tests;
