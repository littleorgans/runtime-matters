mod config;
mod runner;
mod state;

pub use config::DaemonConfig;
pub use runner::run_daemon;

pub(crate) use state::ServerState;

#[cfg(test)]
use {
    crate::reconcile,
    lilo_rm_core::{
        CaptureError, CaptureRequest, CaptureResponse, KillRequest, Lifecycle, LogAvailability,
        LogsUnavailableReason, LostEvidence, NudgeFailureReason, NudgeOutcome, NudgeRequest,
        NudgeResponse, RuntimeExit, RuntimeSignal, ShimReady, StatusFilter,
    },
    rtm_store::{LifecycleStore, StoreConfig},
    std::path::PathBuf,
    uuid::Uuid,
};

#[cfg(test)]
mod tests;
