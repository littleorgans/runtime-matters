use anyhow::{Context, Result};
use rtm_core::{
    RuntimeResponse, RuntimeRpc, ShimExit, ShimReady, SpawnRequest, read_json_line, write_json_line,
};
use tokio::io::BufReader;
use tokio::net::UnixStream;
use tokio::process::Command;

use crate::server::{DaemonConfig, runtime_command_path};

pub async fn launch_shim(config: &DaemonConfig, request: &SpawnRequest) -> Result<()> {
    Command::new(&config.shim_path)
        .arg("__shim")
        .arg("--session-id")
        .arg(request.session_id.to_string())
        .arg("--runtime")
        .arg(request.runtime.as_str())
        .env("RTM_SOCKET_PATH", &config.socket_path)
        .spawn()
        .with_context(|| format!("failed to spawn shim {}", config.shim_path.display()))?;
    Ok(())
}

pub async fn send_ready(socket_path: &std::path::Path, ready: ShimReady) -> Result<()> {
    send_shim_rpc(socket_path, RuntimeRpc::ShimReady { ready }, "ShimReady").await
}

pub async fn send_exit(socket_path: &std::path::Path, exit: ShimExit) -> Result<()> {
    send_shim_rpc(socket_path, RuntimeRpc::ShimExit { exit }, "ShimExit").await
}

async fn send_shim_rpc(
    socket_path: &std::path::Path,
    rpc: RuntimeRpc,
    label: &'static str,
) -> Result<()> {
    let stream = UnixStream::connect(socket_path)
        .await
        .with_context(|| format!("failed to connect to {}", socket_path.display()))?;
    let (read_half, mut write_half) = stream.into_split();
    write_json_line(&mut write_half, &rpc).await?;

    let mut reader = BufReader::new(read_half);
    match read_json_line(&mut reader).await? {
        RuntimeResponse::Ack => Ok(()),
        response => anyhow::bail!("unexpected {label} response: {response:?}"),
    }
}

pub fn runtime_command(runtime: rtm_core::RuntimeKind) -> tokio::process::Command {
    tokio::process::Command::new(runtime_command_path(runtime))
}
