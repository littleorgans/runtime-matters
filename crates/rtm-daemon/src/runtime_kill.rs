use std::time::{Duration, Instant};

use anyhow::Result;
use lilo_rm_core::{IsolationPolicy, KillOutcome, KillRequest, Lifecycle, RuntimeSignal};

use crate::{
    docker_runtime::{self, DockerContainerLiveness},
    error::RuntimeFailure,
    server::ServerState,
};

pub(crate) async fn kill_runtime(state: &ServerState, request: KillRequest) -> Result<KillOutcome> {
    let lifecycle = state
        .store()
        .get(request.session_id)
        .await?
        .ok_or_else(|| RuntimeFailure::session_not_found(request.session_id))?;
    match lifecycle.isolation {
        IsolationPolicy::Host => kill_host_runtime(state, request, lifecycle).await,
        IsolationPolicy::Docker(_) => kill_docker_runtime(state, request).await,
    }
}

async fn kill_host_runtime(
    state: &ServerState,
    request: KillRequest,
    lifecycle: Lifecycle,
) -> Result<KillOutcome> {
    let runtime_pid = lifecycle
        .runtime_pid
        .ok_or_else(|| RuntimeFailure::session_not_found(request.session_id))?;
    let outcome = rtm_platform::signal::send_signal_for_kill(runtime_pid, request.signal)?;
    if matches!(outcome, KillOutcome::AlreadyExited) {
        return Ok(outcome);
    }
    let deadline = Instant::now() + Duration::from_secs(request.grace_secs);

    while Instant::now() < deadline {
        if state.is_terminal(request.session_id).await
            || !rtm_platform::process::pid_alive(runtime_pid)
        {
            return Ok(outcome);
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    if rtm_platform::process::pid_alive(runtime_pid) && request.signal != RuntimeSignal::Kill {
        rtm_platform::signal::send_signal(runtime_pid, RuntimeSignal::Kill)?;
    }
    Ok(outcome)
}

async fn kill_docker_runtime(state: &ServerState, request: KillRequest) -> Result<KillOutcome> {
    let outcome = docker_runtime::kill_container(request.session_id, request.signal).await?;
    if matches!(outcome, KillOutcome::AlreadyExited) {
        return Ok(outcome);
    }
    let deadline = Instant::now() + Duration::from_secs(request.grace_secs);

    while Instant::now() < deadline {
        if state.is_terminal(request.session_id).await
            || !docker_runtime::DockerCliRuntime
                .running(request.session_id)
                .await?
        {
            return Ok(outcome);
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    if docker_runtime::DockerCliRuntime
        .running(request.session_id)
        .await?
        && request.signal != RuntimeSignal::Kill
    {
        docker_runtime::kill_container(request.session_id, RuntimeSignal::Kill).await?;
    }
    Ok(outcome)
}
