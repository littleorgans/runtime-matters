pub mod admin;
pub mod error;
pub mod launcher;
pub mod mcp;
pub mod proto;
pub mod tool_contracts;
pub mod types;
mod version;

pub use admin::{KillByPidRequest, KillByPidResponse, StatusFilter, StatusResponse, WatcherCounts};
pub use error::{ProtocolError, RuntimeKindParseError};
pub use launcher::{LaunchEnv, LaunchSpec, LauncherError, RuntimeLauncher};
pub use mcp::{
    JsonRpcError, JsonRpcRequest, JsonRpcResponse, MCP_PROTOCOL_VERSION, McpBridgeRequest,
    McpBridgeResponse, json_rpc_error, json_rpc_failure, json_rpc_result, tool_error, tool_success,
};
pub use proto::{RuntimeResponse, RuntimeRpc, StatusRequest, read_json_line, write_json_line};
pub use types::{
    KillRequest, Lifecycle, LifecycleState, LostEvidence, NudgeRequest, RuntimeEvent, RuntimeExit,
    RuntimeKind, RuntimeSignal, RuntimeSignalParseError, ShimExit, ShimLaunchRequest, ShimReady,
    SpawnRequest, TerminationEvidence, TmuxPane, TmuxPaneParseError,
};
pub use version::{VersionInfo, version_info};
