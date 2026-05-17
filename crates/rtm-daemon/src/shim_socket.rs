use anyhow::{Context, Result};
use rtm_core::{
    LaunchEnv, LaunchSpec, RuntimeResponse, RuntimeRpc, ShimExit, ShimLaunchRequest, ShimReady,
    SpawnRequest, SpawnTarget, read_json_line, read_json_line_blocking, write_json_line,
    write_json_line_blocking,
};
use std::io::BufReader as StdBufReader;
use std::os::unix::net::UnixStream as StdUnixStream;
use tokio::io::BufReader;
use tokio::net::UnixStream;
use tokio::process::Command;

use crate::server::DaemonConfig;

pub async fn launch_shim(config: &DaemonConfig, request: &SpawnRequest) -> Result<()> {
    let env = shim_env(config);
    match &request.target {
        SpawnTarget::Tmux(target) => {
            let argv = shim_argv(config, request);
            rtm_platform::tmux::TmuxGateway::respawn_pane(&target.address, &argv, &env)
                .await
                .with_context(|| format!("failed to respawn tmux pane {}", target.address))
        }
        SpawnTarget::Headless(_) => launch_headless_shim(config, request, &env),
    }
}

fn launch_headless_shim(
    config: &DaemonConfig,
    request: &SpawnRequest,
    env: &[LaunchEnv],
) -> Result<()> {
    let mut command = Command::new(&config.shim_path);
    command
        .arg("__shim")
        .arg("--session-id")
        .arg(request.session_id.to_string());
    for entry in env {
        command.env(&entry.key, &entry.value);
    }
    command
        .spawn()
        .with_context(|| format!("failed to spawn shim {}", config.shim_path.display()))?;
    Ok(())
}

fn shim_argv(config: &DaemonConfig, request: &SpawnRequest) -> Vec<String> {
    vec![
        config.shim_path.to_string_lossy().into_owned(),
        "__shim".to_owned(),
        "--session-id".to_owned(),
        request.session_id.to_string(),
    ]
}

fn shim_env(config: &DaemonConfig) -> Vec<LaunchEnv> {
    vec![LaunchEnv {
        key: "RTM_SOCKET_PATH".to_owned(),
        value: config.socket_path.to_string_lossy().into_owned(),
    }]
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
    launch_from_response(read_json_line(&mut reader).await?)
}

pub fn request_launch_blocking(
    socket_path: &std::path::Path,
    request: ShimLaunchRequest,
) -> Result<LaunchSpec> {
    let mut stream = StdUnixStream::connect(socket_path)
        .with_context(|| format!("failed to connect to {}", socket_path.display()))?;
    write_json_line_blocking(&mut stream, &RuntimeRpc::ShimLaunch { request })?;

    let mut reader = StdBufReader::new(stream);
    launch_from_response(read_json_line_blocking(&mut reader)?)
}

pub async fn send_ready(socket_path: &std::path::Path, ready: ShimReady) -> Result<()> {
    send_shim_rpc(socket_path, RuntimeRpc::ShimReady { ready }, "ShimReady").await
}

pub fn send_ready_blocking(socket_path: &std::path::Path, ready: ShimReady) -> Result<()> {
    send_shim_rpc_blocking(socket_path, RuntimeRpc::ShimReady { ready }, "ShimReady")
}

pub async fn send_exit(socket_path: &std::path::Path, exit: ShimExit) -> Result<()> {
    send_shim_rpc(socket_path, RuntimeRpc::ShimExit { exit }, "ShimExit").await
}

pub fn send_exit_blocking(socket_path: &std::path::Path, exit: ShimExit) -> Result<()> {
    send_shim_rpc_blocking(socket_path, RuntimeRpc::ShimExit { exit }, "ShimExit")
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
    ack_from_response(read_json_line(&mut reader).await?, label)
}

fn send_shim_rpc_blocking(
    socket_path: &std::path::Path,
    rpc: RuntimeRpc,
    label: &'static str,
) -> Result<()> {
    let mut stream = StdUnixStream::connect(socket_path)
        .with_context(|| format!("failed to connect to {}", socket_path.display()))?;
    write_json_line_blocking(&mut stream, &rpc)?;

    let mut reader = StdBufReader::new(stream);
    ack_from_response(read_json_line_blocking(&mut reader)?, label)
}

fn launch_from_response(response: RuntimeResponse) -> Result<LaunchSpec> {
    match response {
        RuntimeResponse::ShimLaunch { launch } => Ok(launch),
        RuntimeResponse::Error { message } => anyhow::bail!(message),
        response => anyhow::bail!("unexpected ShimLaunch response: {response:?}"),
    }
}

fn ack_from_response(response: RuntimeResponse, label: &'static str) -> Result<()> {
    match response {
        RuntimeResponse::Ack => Ok(()),
        response => anyhow::bail!("unexpected {label} response: {response:?}"),
    }
}
