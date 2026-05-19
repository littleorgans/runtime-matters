mod doctor;
mod error;
pub mod event_channel;
mod event_log;
mod handler;
mod mcp_bridge;
mod reconcile;
pub mod server;
pub mod shim_socket;
pub mod socket;
mod spawn_preflight;
pub(crate) mod version;

pub use reconcile::ReconcileConfig;
pub use server::{DaemonConfig, run_daemon};
