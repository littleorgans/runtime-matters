use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use rtm_core::{Lifecycle, LifecycleState, RuntimeEvent, RuntimeKind, ShimReady, SpawnRequest};
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

    socket::remove_socket_file(&config.socket_path)?;
    Ok(())
}

pub(crate) struct ServerState {
    config: DaemonConfig,
    events: Mutex<Vec<RuntimeEvent>>,
    lifecycles: Mutex<HashMap<Uuid, Lifecycle>>,
    pending_ready: Mutex<HashMap<Uuid, oneshot::Sender<ShimReady>>>,
}

impl ServerState {
    fn new(config: DaemonConfig) -> Self {
        Self {
            config,
            events: Mutex::new(Vec::new()),
            lifecycles: Mutex::new(HashMap::new()),
            pending_ready: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn config(&self) -> &DaemonConfig {
        &self.config
    }

    pub(crate) async fn begin_spawn(
        &self,
        session_id: Uuid,
    ) -> Result<oneshot::Receiver<ShimReady>> {
        if self.lifecycles.lock().await.contains_key(&session_id) {
            bail!("session {session_id} is already running");
        }

        let (sender, receiver) = oneshot::channel();
        let previous = self.pending_ready.lock().await.insert(session_id, sender);
        if previous.is_some() {
            bail!("session {session_id} already has a pending shim");
        }
        Ok(receiver)
    }

    pub(crate) async fn cancel_spawn(&self, session_id: Uuid) {
        self.pending_ready.lock().await.remove(&session_id);
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
        &self,
        request: &SpawnRequest,
        ready: ShimReady,
    ) -> Result<(Lifecycle, RuntimeEvent)> {
        let lifecycle = Lifecycle {
            session_id: request.session_id,
            runtime: request.runtime,
            state: LifecycleState::Running,
            runtime_pid: ready.runtime_pid,
            start_time: ready.start_time,
            tmux_pane: None,
        };
        let event = event_channel::running_event(&lifecycle);

        self.lifecycles
            .lock()
            .await
            .insert(request.session_id, lifecycle.clone());
        self.events.lock().await.push(event.clone());
        Ok((lifecycle, event))
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
}

pub fn resolve_claude_path() -> PathBuf {
    if let Some(path) = std::env::var_os("RTM_CLAUDE_PATH") {
        return PathBuf::from(path);
    }

    // Pass 1 shortcut: this hardcoded Claude resolver is removed in Pass 3,
    // when RuntimeLauncher dispatch owns runtime specific launch behavior.
    option_env!("RTM_COMPILED_CLAUDE_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("claude"))
}

pub(crate) fn runtime_command_path(runtime: RuntimeKind) -> PathBuf {
    match runtime {
        RuntimeKind::Claude => resolve_claude_path(),
    }
}
