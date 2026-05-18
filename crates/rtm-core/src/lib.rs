//! Runtime Matters core protocol types and JSON line transport helpers.
//!
//! This crate is the stable contract shared by `rtm` clients and rtmd. The
//! daemon, CLI, platform, launcher, and store crates remain private
//! implementation details.
//!
//! ## Events contract
//!
//! v0.2 events use [`RuntimeRpc::Events`] and
//! [`RuntimeResponse::Events { events }`](RuntimeResponse::Events). The response
//! is the current daemon process vector in append order. rtmd appends
//! [`RuntimeEvent::Running`] when shim ready is stored, then appends
//! [`RuntimeEvent::Terminated`] or [`RuntimeEvent::Lost`] when exit or loss
//! evidence is observed.
//!
//! Events are kept only in the current daemon process memory. There is no v0.2
//! cursor, retention window, sqlite replay, or limit policy. Clients such as
//! session-matters should poll, filter to their session set, and dedupe by
//! session id plus full event content. Cursor based
//! `Events { since } -> { cursor, events }` support is deferred to v0.3.

pub mod admin;
pub mod error;
pub mod launcher;
pub mod mcp;
pub mod proto;
pub mod spawn_context;
pub mod tool_contracts;
pub mod types;
mod version;

pub use admin::{
    DoctorResponse, KillByPidRequest, KillByPidResponse, LauncherStatus, LifecycleCounts,
    MigrationState, RecentLostEvent, StatusFilter, StatusResponse, TmuxStatus, WatcherCounts,
};
pub use error::{ErrorCode, ProtocolError, RuntimeKindParseError};
pub use launcher::{LaunchEnv, LaunchSpec, LauncherError, RuntimeLauncher};
pub use mcp::{
    JsonRpcError, JsonRpcRequest, JsonRpcResponse, MCP_PROTOCOL_VERSION, McpBridgeRequest,
    McpBridgeResponse, json_rpc_error, json_rpc_failure, json_rpc_result, tool_error, tool_success,
};
pub use proto::{
    RuntimeResponse, RuntimeRpc, StatusRequest, read_json_line, read_json_line_blocking,
    write_json_line, write_json_line_blocking,
};
pub use spawn_context::{
    CALLER_ENV_DENYLIST, CALLER_ENV_DENYLIST_PREFIXES, capture_caller_cwd, capture_caller_env,
    capture_env_from, capture_env_from_os, launcher_probe_cwd,
};
pub use types::{
    HeadlessSpawnTarget, KillRequest, Lifecycle, LifecycleState, LostEvidence, NudgeFailureReason,
    NudgeOutcome, NudgeRequest, NudgeResponse, RuntimeEvent, RuntimeExit, RuntimeKind,
    RuntimeSignal, RuntimeSignalParseError, ShimExit, ShimLaunchRequest, ShimReady, SpawnRequest,
    SpawnTarget, SpawnTargetParseError, TerminationEvidence, TmuxAddress, TmuxAddressParseError,
    TmuxSpawnTarget, ValidateTargetOutcome, ValidateTargetRequest, ValidateTargetResponse,
};
pub use version::{
    RUNTIME_PROTOCOL_CAPABILITIES, RUNTIME_PROTOCOL_VERSION, RuntimeCapability, VersionInfo,
    version_info,
};
