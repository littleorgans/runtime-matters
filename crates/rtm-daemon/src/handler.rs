use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use rtm_core::{RuntimeResponse, RuntimeRpc, StatusRequest, read_json_line, write_json_line};
use tokio::io::BufReader;
use tokio::net::UnixStream;
use tokio::sync::broadcast;

use crate::{server::ServerState, shim_socket};

pub(crate) async fn handle_connection(
    stream: UnixStream,
    state: Arc<ServerState>,
    shutdown_tx: broadcast::Sender<()>,
) -> Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    let response = match read_json_line::<_, RuntimeRpc>(&mut reader).await {
        Ok(rpc) => handle_rpc(rpc, state).await,
        Err(error) => RuntimeResponse::Error {
            message: error.to_string(),
        },
    };
    let should_stop = matches!(response, RuntimeResponse::Stopping);

    write_json_line(&mut write_half, &response).await?;
    if should_stop {
        let _ = shutdown_tx.send(());
    }
    Ok(())
}

async fn handle_rpc(rpc: RuntimeRpc, state: Arc<ServerState>) -> RuntimeResponse {
    match handle_rpc_result(rpc, state).await {
        Ok(response) => response,
        Err(error) => RuntimeResponse::Error {
            message: error.to_string(),
        },
    }
}

async fn handle_rpc_result(rpc: RuntimeRpc, state: Arc<ServerState>) -> Result<RuntimeResponse> {
    match rpc {
        RuntimeRpc::Spawn { request } => {
            let ready_rx = state.begin_spawn(&request).await?;
            if let Err(error) = shim_socket::launch_shim(state.config(), &request).await {
                state.cancel_spawn(request.session_id).await;
                return Err(error);
            }

            let ready = tokio::time::timeout(Duration::from_secs(10), ready_rx)
                .await
                .context("timed out waiting for ShimReady")?
                .context("shim ready channel closed")?;
            let (lifecycle, event) = state.record_running(&request, ready).await?;
            Ok(RuntimeResponse::Spawned { lifecycle, event })
        }
        RuntimeRpc::Kill { request } => {
            state.kill_runtime(request).await?;
            Ok(RuntimeResponse::Ack)
        }
        RuntimeRpc::Status {
            request: StatusRequest { session_id },
        } => Ok(RuntimeResponse::Status {
            lifecycles: state.status(session_id).await,
        }),
        RuntimeRpc::Events => Ok(RuntimeResponse::Events {
            events: state.events().await,
        }),
        RuntimeRpc::Stop => Ok(RuntimeResponse::Stopping),
        RuntimeRpc::ShimReady { ready } => {
            state.complete_shim_ready(ready).await?;
            Ok(RuntimeResponse::Ack)
        }
        RuntimeRpc::ShimExit { exit } => {
            let _ = state.record_shim_exit(exit).await?;
            Ok(RuntimeResponse::Ack)
        }
    }
}
