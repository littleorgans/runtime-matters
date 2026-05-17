use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Lifecycle;

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
    pub runtime: Option<String>,
    pub state: Option<String>,
}

impl StatusFilter {
    pub const fn empty() -> Self {
        Self {
            session_id: None,
            runtime: None,
            state: None,
        }
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
