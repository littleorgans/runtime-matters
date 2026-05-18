use std::sync::Arc;

use anyhow::Result;
use chrono::{Duration, Utc};
use lilo_rm_core::{
    DoctorResponse, HeadlessSpawnTarget, LauncherStatus, SpawnRequest, SpawnTarget, TmuxStatus,
};
use uuid::Uuid;

use crate::{server::ServerState, socket};

const RECENT_LOST_WINDOW: Duration = Duration::hours(24);

pub(crate) async fn collect(state: Arc<ServerState>) -> Result<DoctorResponse> {
    Ok(DoctorResponse {
        version: crate::version::runtime_version_info(),
        socket_path: socket::display_socket_path(&state.config().socket_path),
        uptime_secs: state.uptime_secs(),
        sqlite: state.store().migration_state().await?,
        lifecycles: state.store().lifecycle_counts().await?,
        watchers: state.watcher_counts().await,
        launchers: launcher_statuses(),
        tmux: tmux_status().await,
        last_probe_sweep: state.store().last_probe_sweep().await?,
        recent_lost: state
            .store()
            .recent_lost_since(Utc::now() - RECENT_LOST_WINDOW)
            .await?,
    })
}

fn launcher_statuses() -> Vec<LauncherStatus> {
    rtm_launchers::registered_launchers()
        .into_iter()
        .map(launcher_status)
        .collect()
}

fn launcher_status(launcher: &'static dyn lilo_rm_core::RuntimeLauncher) -> LauncherStatus {
    let runtime = launcher.kind();
    let request = SpawnRequest {
        session_id: Uuid::nil(),
        runtime: runtime.clone(),
        env: Vec::new(),
        cwd: lilo_rm_core::launcher_probe_cwd(),
        target: SpawnTarget::Headless(HeadlessSpawnTarget {}),
    };
    match launcher.argv(&request) {
        Ok(argv) => LauncherStatus {
            runtime: runtime.to_string(),
            command: argv.first().cloned(),
            error: None,
        },
        Err(error) => LauncherStatus {
            runtime: runtime.to_string(),
            command: None,
            error: Some(error.to_string()),
        },
    }
}

async fn tmux_status() -> TmuxStatus {
    match rtm_platform::tmux::TmuxGateway::version().await {
        Ok(Some(version)) => TmuxStatus {
            available: true,
            version: Some(version),
            error: None,
        },
        Ok(None) => TmuxStatus {
            available: false,
            version: None,
            error: None,
        },
        Err(error) => TmuxStatus {
            available: false,
            version: None,
            error: Some(error.to_string()),
        },
    }
}
