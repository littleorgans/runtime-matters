use std::path::Path;

use anyhow::Result;
use rtm_core::{RuntimeKind, RuntimeResponse, RuntimeRpc, SpawnRequest, SpawnTarget};
use uuid::Uuid;

pub async fn spawn_runtime(
    socket_path: &Path,
    session_id: Uuid,
    runtime: RuntimeKind,
    target: SpawnTarget,
) -> Result<RuntimeResponse> {
    rtm_cli::shared::request(
        socket_path,
        RuntimeRpc::Spawn {
            request: SpawnRequest {
                session_id,
                runtime,
                env: Vec::new(),
                cwd: None,
                target,
            },
        },
    )
    .await
}
