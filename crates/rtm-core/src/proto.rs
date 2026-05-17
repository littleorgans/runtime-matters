use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};
use uuid::Uuid;

use crate::{
    KillByPidRequest, KillByPidResponse, KillRequest, LaunchSpec, Lifecycle, McpBridgeRequest,
    McpBridgeResponse, NudgeRequest, ProtocolError, RuntimeEvent, ShimExit, ShimLaunchRequest,
    ShimReady, SpawnRequest, StatusFilter, WatcherCounts,
};

#[derive(Clone, Debug, serde::Deserialize, Eq, PartialEq, serde::Serialize)]
pub struct StatusRequest {
    pub session_id: Option<Uuid>,
    #[serde(default)]
    pub runtime: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
}

impl From<StatusRequest> for StatusFilter {
    fn from(request: StatusRequest) -> Self {
        Self {
            session_id: request.session_id,
            runtime: request.runtime,
            state: request.state,
        }
    }
}

#[derive(Clone, Debug, serde::Deserialize, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum RuntimeRpc {
    Spawn { request: SpawnRequest },
    Kill { request: KillRequest },
    KillByPid { request: KillByPidRequest },
    Nudge { request: NudgeRequest },
    Status { request: StatusRequest },
    Version,
    Watchers,
    Doctor,
    Events,
    Stop,
    McpBridge { request: McpBridgeRequest },
    ShimLaunch { request: ShimLaunchRequest },
    ShimReady { ready: ShimReady },
    ShimExit { exit: ShimExit },
}

#[derive(Clone, Debug, serde::Deserialize, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum RuntimeResponse {
    Spawned {
        lifecycle: Lifecycle,
        event: RuntimeEvent,
    },
    Status {
        lifecycles: Vec<Lifecycle>,
    },
    KillByPid {
        response: KillByPidResponse,
    },
    Version {
        version: crate::VersionInfo,
    },
    Watchers {
        watchers: WatcherCounts,
    },
    Doctor {
        doctor: crate::DoctorResponse,
    },
    Events {
        events: Vec<RuntimeEvent>,
    },
    McpBridge {
        response: McpBridgeResponse,
    },
    ShimLaunch {
        launch: LaunchSpec,
    },
    Ack,
    Stopping,
    Error {
        message: String,
    },
}

pub async fn read_json_line<R, T>(reader: &mut R) -> Result<T, ProtocolError>
where
    R: AsyncBufRead + Unpin,
    T: DeserializeOwned,
{
    let mut line = String::new();
    let read = reader.read_line(&mut line).await?;
    if read == 0 {
        return Err(ProtocolError::Eof);
    }
    Ok(serde_json::from_str(line.trim_end())?)
}

pub async fn write_json_line<W, T>(writer: &mut W, message: &T) -> Result<(), ProtocolError>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let mut bytes = serde_json::to_vec(message)?;
    bytes.push(b'\n');
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}
