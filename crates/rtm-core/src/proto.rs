use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};
use uuid::Uuid;

use crate::{
    KillRequest, LaunchSpec, Lifecycle, ProtocolError, RuntimeEvent, ShimExit, ShimLaunchRequest,
    ShimReady, SpawnRequest,
};

#[derive(Clone, Debug, serde::Deserialize, Eq, PartialEq, serde::Serialize)]
pub struct StatusRequest {
    pub session_id: Option<Uuid>,
}

#[derive(Clone, Debug, serde::Deserialize, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum RuntimeRpc {
    Spawn { request: SpawnRequest },
    Kill { request: KillRequest },
    Status { request: StatusRequest },
    Events,
    Stop,
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
    Events {
        events: Vec<RuntimeEvent>,
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
