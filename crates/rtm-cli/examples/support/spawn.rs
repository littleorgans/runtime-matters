use std::path::Path;

use anyhow::{Context, Result};
use rtm_core::{RuntimeKind, RuntimeResponse, RuntimeRpc, SpawnRequest, SpawnTarget};
use uuid::Uuid;

pub async fn spawn_runtime(
    socket_path: &Path,
    session_id: Uuid,
    runtime: RuntimeKind,
    target: SpawnTarget,
) -> Result<RuntimeResponse> {
    let cwd = rtm_core::capture_caller_cwd().context("failed to capture caller cwd")?;
    let env = rtm_core::capture_caller_env();
    rtm_cli::shared::request(
        socket_path,
        RuntimeRpc::Spawn {
            request: SpawnRequest {
                session_id,
                runtime,
                env,
                cwd,
                target,
            },
        },
    )
    .await
}
