use anyhow::{Context, Result};
use rtm_core::{
    LaunchSpec, RuntimeResponse, RuntimeRpc, ShimExit, ShimLaunchRequest, ShimReady, SpawnRequest,
    read_json_line, write_json_line,
};
use tokio::io::BufReader;
use tokio::net::UnixStream;
use tokio::process::Command;

use crate::server::DaemonConfig;

pub async fn launch_shim(config: &DaemonConfig, request: &SpawnRequest) -> Result<()> {
    Command::new(&config.shim_path)
        .arg("__shim")
        .arg("--session-id")
        .arg(request.session_id.to_string())
        .env("RTM_SOCKET_PATH", &config.socket_path)
        .spawn()
        .with_context(|| format!("failed to spawn shim {}", config.shim_path.display()))?;
    Ok(())
}

pub async fn request_launch(
    socket_path: &std::path::Path,
    request: ShimLaunchRequest,
) -> Result<LaunchSpec> {
    let stream = UnixStream::connect(socket_path)
        .await
        .with_context(|| format!("failed to connect to {}", socket_path.display()))?;
    let (read_half, mut write_half) = stream.into_split();
    write_json_line(&mut write_half, &RuntimeRpc::ShimLaunch { request }).await?;

    let mut reader = BufReader::new(read_half);
    match read_json_line(&mut reader).await? {
        RuntimeResponse::ShimLaunch { launch } => Ok(launch),
        RuntimeResponse::Error { message } => anyhow::bail!(message),
        response => anyhow::bail!("unexpected ShimLaunch response: {response:?}"),
    }
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
