use std::io::{BufRead, Write};
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};
use uuid::Uuid;

use crate::{
    ErrorCode, KillByPidRequest, KillByPidResponse, KillRequest, LaunchSpec, Lifecycle,
    McpBridgeRequest, McpBridgeResponse, NudgeRequest, NudgeResponse, ProtocolError, RuntimeEvent,
    ShimExit, ShimLaunchRequest, ShimReady, SpawnRequest, StatusFilter, ValidateTargetRequest,
    ValidateTargetResponse, WatcherCounts,
};

pub type EventCursor = u64;

pub const EVENT_LOG_RETENTION_MIN_AGE_SECS: u64 = 7 * 24 * 60 * 60;
pub const EVENT_LOG_RETENTION_MIN_EVENTS: usize = 10_000;
/// Maximum single Events long poll wait window.
///
/// Requests above this ceiling are clamped rather than rejected.
pub const EVENT_WAIT_MAX_MS: u32 = 60_000;

#[derive(Clone, Copy, Debug, Default, serde::Deserialize, Eq, PartialEq, serde::Serialize)]
pub struct EventsRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<EventCursor>,
    /// Optional long poll window in milliseconds. `None` and `0` return immediately.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_ms: Option<u32>,
}

pub const fn clamped_event_wait_ms(wait_ms: Option<u32>) -> u32 {
    match wait_ms {
        Some(value) if value < EVENT_WAIT_MAX_MS => value,
        Some(_) => EVENT_WAIT_MAX_MS,
        None => 0,
    }
}

#[derive(Clone, Debug, serde::Deserialize, Eq, PartialEq, serde::Serialize)]
pub struct StatusRequest {
    pub session_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub session_ids: Vec<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_since: Option<DateTime<Utc>>,
    #[serde(default)]
    pub runtime: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
}

impl From<StatusRequest> for StatusFilter {
    fn from(request: StatusRequest) -> Self {
        Self {
            session_id: request.session_id,
            session_ids: request.session_ids,
            updated_since: request.updated_since,
            runtime: request.runtime,
            state: request.state,
        }
    }
}

impl From<StatusFilter> for StatusRequest {
    fn from(filter: StatusFilter) -> Self {
        Self {
            session_id: filter.session_id,
            session_ids: filter.session_ids,
            updated_since: filter.updated_since,
            runtime: filter.runtime,
            state: filter.state,
        }
    }
}

#[derive(Clone, Debug, serde::Deserialize, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum RuntimeRpc {
    Spawn {
        request: SpawnRequest,
    },
    ValidateTarget {
        request: ValidateTargetRequest,
    },
    Kill {
        request: KillRequest,
    },
    KillByPid {
        request: KillByPidRequest,
    },
    Nudge {
        request: NudgeRequest,
    },
    Capture {
        request: crate::CaptureRequest,
    },
    Status {
        request: StatusRequest,
    },
    Version,
    Watchers,
    Doctor,
    Events {
        #[serde(default, flatten)]
        request: EventsRequest,
    },
    Stop,
    McpBridge {
        request: McpBridgeRequest,
    },
    ShimLaunch {
        request: ShimLaunchRequest,
    },
    ShimReady {
        ready: ShimReady,
    },
    ShimExit {
        exit: ShimExit,
    },
}

#[derive(Clone, Debug, serde::Deserialize, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum RuntimeResponse {
    Spawned {
        lifecycle: Lifecycle,
        event: RuntimeEvent,
        log_dir: Option<PathBuf>,
        stdout_path: Option<PathBuf>,
        stderr_path: Option<PathBuf>,
    },
    ValidateTarget {
        response: ValidateTargetResponse,
    },
    Status {
        lifecycles: Vec<Lifecycle>,
    },
    KillByPid {
        response: KillByPidResponse,
    },
    Nudge {
        response: NudgeResponse,
    },
    Capture {
        response: crate::CaptureResponse,
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
    /// Events in daemon append order.
    Events {
        events: Vec<RuntimeEvent>,
        cursor: EventCursor,
    },
    CursorExpired {
        oldest: EventCursor,
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
        code: ErrorCode,
        message: String,
    },
}

impl RuntimeResponse {
    pub fn error(code: ErrorCode, message: impl Into<String>) -> Self {
        Self::Error {
            code,
            message: message.into(),
        }
    }
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
    parse_json_line(&line)
}

pub async fn write_json_line<W, T>(writer: &mut W, message: &T) -> Result<(), ProtocolError>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let bytes = json_line_bytes(message)?;
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

pub fn read_json_line_blocking<R, T>(reader: &mut R) -> Result<T, ProtocolError>
where
    R: BufRead,
    T: DeserializeOwned,
{
    let mut line = String::new();
    let read = reader.read_line(&mut line)?;
    if read == 0 {
        return Err(ProtocolError::Eof);
    }
    parse_json_line(&line)
}

pub fn write_json_line_blocking<W, T>(writer: &mut W, message: &T) -> Result<(), ProtocolError>
where
    W: Write,
    T: Serialize,
{
    let bytes = json_line_bytes(message)?;
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(())
}

fn parse_json_line<T>(line: &str) -> Result<T, ProtocolError>
where
    T: DeserializeOwned,
{
    Ok(serde_json::from_str(line.trim_end())?)
}

fn json_line_bytes<T>(message: &T) -> Result<Vec<u8>, ProtocolError>
where
    T: Serialize,
{
    let mut bytes = serde_json::to_vec(message)?;
    bytes.push(b'\n');
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamped_event_wait_ms_applies_ceiling_and_default() {
        assert_eq!(clamped_event_wait_ms(None), 0);
        assert_eq!(clamped_event_wait_ms(Some(500)), 500);
        assert_eq!(
            clamped_event_wait_ms(Some(EVENT_WAIT_MAX_MS)),
            EVENT_WAIT_MAX_MS
        );
        assert_eq!(
            clamped_event_wait_ms(Some(EVENT_WAIT_MAX_MS + 1)),
            EVENT_WAIT_MAX_MS
        );
    }
}
