use serde::{Deserialize, Serialize};
use uuid::Uuid;

use chrono::{DateTime, Utc};

use crate::{Lifecycle, LostEvidence, VersionInfo};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KillByPidRequest {
    pub pid: u32,
    pub signal: i32,
    pub grace_secs: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KillByPidResponse {
    pub pid: u32,
    pub signal: i32,
    pub killed_after_grace: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StatusFilter {
    pub session_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub session_ids: Vec<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_since: Option<DateTime<Utc>>,
    pub runtime: Option<String>,
    pub state: Option<String>,
}

impl StatusFilter {
    pub const fn empty() -> Self {
        Self {
            session_id: None,
            session_ids: Vec::new(),
            updated_since: None,
            runtime: None,
            state: None,
        }
    }

    pub fn requested_session_ids(&self) -> Vec<Uuid> {
        let mut ids = self.session_ids.clone();
        if let Some(session_id) = self.session_id
            && !ids.contains(&session_id)
        {
            ids.push(session_id);
        }
        ids
    }
}

impl Default for StatusFilter {
    fn default() -> Self {
        Self::empty()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StatusResponse {
    pub lifecycles: Vec<Lifecycle>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WatcherCounts {
    pub kqueue_watchers: usize,
    pub shim_sockets: usize,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct LifecycleCounts {
    pub forking: u64,
    pub running: u64,
    pub exited: u64,
    pub lost: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MigrationState {
    pub applied: usize,
    pub total: usize,
    pub applied_descriptions: Vec<String>,
    pub pending_descriptions: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RecentLostEvent {
    pub session_id: Uuid,
    pub evidence: LostEvidence,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LauncherStatus {
    pub runtime: String,
    pub command: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TmuxStatus {
    pub available: bool,
    pub version: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DoctorResponse {
    pub version: VersionInfo,
    pub socket_path: String,
    pub uptime_secs: u64,
    pub sqlite: MigrationState,
    pub lifecycles: LifecycleCounts,
    pub watchers: WatcherCounts,
    pub launchers: Vec<LauncherStatus>,
    pub tmux: TmuxStatus,
    pub last_probe_sweep: Option<DateTime<Utc>>,
    pub recent_lost: Vec<RecentLostEvent>,
}
