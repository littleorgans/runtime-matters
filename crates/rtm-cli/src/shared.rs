use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use lilo_rm_core::{RuntimeEvent, RuntimeResponse, RuntimeRpc, StatusFilter, StatusRequest};
use uuid::Uuid;

pub fn socket_path() -> Result<PathBuf> {
    rtm_daemon::socket::socket_path_from_env()
}

pub async fn request(socket_path: &Path, rpc: RuntimeRpc) -> Result<RuntimeResponse> {
    Ok(lilo_rm_client::request(socket_path, rpc).await?)
}

pub async fn status(socket_path: &Path, session_id: Option<Uuid>) -> Result<RuntimeResponse> {
    status_filtered(
        socket_path,
        StatusFilter {
            session_id,
            session_ids: Vec::new(),
            updated_since: None,
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
            request: StatusRequest::from(filter),
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
