use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use rtm_core::{
    LaunchEnv, RuntimeEvent, RuntimeResponse, RuntimeRpc, StatusFilter, StatusRequest,
    read_json_line, write_json_line,
};
use tokio::io::BufReader;
use tokio::net::UnixStream;
use uuid::Uuid;

pub fn socket_path() -> Result<PathBuf> {
    rtm_daemon::socket::socket_path_from_env()
}

pub fn client_launch_env() -> Vec<LaunchEnv> {
    ["TMUX", "TMUX_PANE"]
        .into_iter()
        .filter_map(|key| {
            std::env::var(key)
                .ok()
                .map(|value| LaunchEnv::new(key, value))
        })
        .collect()
}

pub async fn request(socket_path: &Path, rpc: RuntimeRpc) -> Result<RuntimeResponse> {
    let stream = UnixStream::connect(socket_path)
        .await
        .with_context(|| format!("failed to connect to {}", socket_path.display()))?;
    let (read_half, mut write_half) = stream.into_split();
    write_json_line(&mut write_half, &rpc).await?;

    let mut reader = BufReader::new(read_half);
    let response = read_json_line(&mut reader).await?;
    match response {
        RuntimeResponse::Error { message } => bail!(message),
        other => Ok(other),
    }
}

pub async fn status(socket_path: &Path, session_id: Option<Uuid>) -> Result<RuntimeResponse> {
    status_filtered(
        socket_path,
        StatusFilter {
            session_id,
            runtime: None,
            state: None,
        },
    )
    .await
}

pub async fn status_filtered(socket_path: &Path, filter: StatusFilter) -> Result<RuntimeResponse> {
    request(
        socket_path,
        RuntimeRpc::Status {
            request: StatusRequest {
                session_id: filter.session_id,
                runtime: filter.runtime,
                state: filter.state,
            },
        },
    )
    .await
}

pub async fn events(socket_path: &Path) -> Result<Vec<RuntimeEvent>> {
    match request(socket_path, RuntimeRpc::Events).await? {
        RuntimeResponse::Events { events } => Ok(events),
        other => bail!("unexpected response to events request: {other:?}"),
    }
}

pub async fn wait_for_socket_removed(path: &Path, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !path.exists() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    bail!("socket {} still exists after daemon stop", path.display())
}
