mod doctor;
pub mod event_channel;
mod handler;
mod mcp_bridge;
mod reconcile;
pub mod server;
pub mod shim_socket;
pub mod socket;
pub(crate) mod version;

pub use server::{DaemonConfig, run_daemon};
