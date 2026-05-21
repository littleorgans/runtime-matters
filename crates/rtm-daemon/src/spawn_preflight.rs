use std::sync::Arc;

use anyhow::Result;
use lilo_rm_core::{
    IsolationPolicy, IsolationProfile, KillRequest, RuntimeResponse, RuntimeSignal,
    SpawnConflictKind, SpawnConflictPayload, SpawnRequest,
};

use crate::server::ServerState;
use crate::{
    docker_preflight::{DockerCliInspector, DockerImageInspector},
    error::RuntimeFailure,
};

pub(crate) async fn check(
    state: &Arc<ServerState>,
    request: &SpawnRequest,
) -> Result<Option<RuntimeResponse>> {
    check_with_docker_inspector(state, request, &DockerCliInspector).await
}

async fn check_with_docker_inspector(
    state: &Arc<ServerState>,
    request: &SpawnRequest,
    docker: &impl DockerImageInspector,
) -> Result<Option<RuntimeResponse>> {
    check_isolation_policy(state, request, docker).await?;

    if let Some(lifecycle) = state.store().get(request.session_id).await? {
        return Ok(Some(conflict(SpawnConflictKind::SessionId, lifecycle)));
    }

    let Some(address) = request.target.tmux_address() else {
        return Ok(None);
    };
    let Some(occupant) = state.store().running_tmux_occupant(address).await? else {
        return Ok(None);
    };
    if !request.force {
        return Ok(Some(conflict(
            SpawnConflictKind::TmuxPaneOccupancy,
            occupant,
        )));
    }

    state
        .kill_runtime(KillRequest {
            session_id: occupant.session_id,
            signal: RuntimeSignal::Term,
            grace_secs: 2,
        })
        .await?;
    Ok(None)
}

async fn check_isolation_policy(
    state: &Arc<ServerState>,
    request: &SpawnRequest,
    docker: &impl DockerImageInspector,
) -> Result<()> {
    match &request.isolation {
        IsolationPolicy::Host => Ok(()),
        IsolationPolicy::Docker(profile) => {
            check_docker_profile(state, request, profile, docker).await
        }
    }
}

async fn check_docker_profile(
    state: &Arc<ServerState>,
    request: &SpawnRequest,
    profile: &IsolationProfile,
    docker: &impl DockerImageInspector,
) -> Result<()> {
    match profile.name.as_deref() {
        None
        | Some("default")
        | Some("own-init")
        | Some("allow-root")
        | Some("arm64-manifest-escape") => {
            validate_docker_image_metadata_on_arch(
                state,
                request,
                profile,
                docker,
                std::env::consts::ARCH,
            )
            .await
        }
        Some("pattern-e") | Some("tmux-primary") => Err(unsupported_docker_profile(
            profile,
            "requests unsupported Pattern E",
        )),
        Some("privileged") => Err(unsupported_docker_profile(
            profile,
            "requests privileged execution",
        )),
        Some(_) => Err(unsupported_docker_profile(
            profile,
            "is not an accepted Docker profile",
        )),
    }
}

async fn validate_docker_image_metadata_on_arch(
    state: &Arc<ServerState>,
    request: &SpawnRequest,
    profile: &IsolationProfile,
    docker: &impl DockerImageInspector,
    host_arch: &str,
) -> Result<()> {
    docker.ensure_available().await?;
    let image = state.config().docker_preflight.image_for(request)?;
    let user = docker.image_user(image).await?;
    if image_user_is_root(user.as_deref()) && !docker_root_allowed(state, profile) {
        return Err(RuntimeFailure::docker_image_metadata_unavailable(format!(
            "docker image {image} runs as root"
        )));
    }
    if host_arch == "aarch64" && !docker_manifest_escape_allowed(state, profile) {
        let arm64_available = docker_image_arm64_available(docker, image).await?;
        if !arm64_available {
            return Err(RuntimeFailure::docker_image_metadata_unavailable(format!(
                "docker image {image} does not publish an arm64 manifest"
            )));
        }
    }
    Ok(())
}

async fn docker_image_arm64_available(
    docker: &impl DockerImageInspector,
    image: &str,
) -> Result<bool> {
    match docker.arm64_manifest_available(image).await {
        Ok(available) => Ok(available),
        Err(manifest_error) => match docker.image_architecture(image).await {
            Ok(arch) => Ok(arch == "arm64"),
            Err(local_error) if is_docker_image_unavailable(&local_error) => Err(local_error),
            Err(_) => Err(manifest_error),
        },
    }
}

fn is_docker_image_unavailable(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<RuntimeFailure>()
        .is_some_and(|failure| matches!(failure, RuntimeFailure::DockerImageUnavailable { .. }))
}

fn docker_root_allowed(state: &Arc<ServerState>, profile: &IsolationProfile) -> bool {
    state.config().docker_preflight.allows_root_image_user()
        || profile.name.as_deref() == Some("allow-root")
}

fn docker_manifest_escape_allowed(state: &Arc<ServerState>, profile: &IsolationProfile) -> bool {
    state
        .config()
        .docker_preflight
        .allows_arm64_manifest_escape()
        || profile.name.as_deref() == Some("arm64-manifest-escape")
}

fn image_user_is_root(user: Option<&str>) -> bool {
    let Some(user) = user else {
        return true;
    };
    let primary = user.split(':').next().unwrap_or(user).trim();
    primary.is_empty() || primary == "0" || primary == "root"
}

fn unsupported_docker_profile(profile: &IsolationProfile, reason: &str) -> anyhow::Error {
    RuntimeFailure::unsupported_isolation_policy(format!(
        "{} ({reason})",
        IsolationPolicy::Docker(profile.clone())
    ))
}

fn conflict(kind: SpawnConflictKind, lifecycle: lilo_rm_core::Lifecycle) -> RuntimeResponse {
    RuntimeResponse::SpawnConflict(SpawnConflictPayload { kind, lifecycle })
}

#[cfg(test)]
mod tests;
