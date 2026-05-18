//! Async Unix socket client for the public rtmd JSON line contract.
//!
//! `lilo-rm-client` owns connection setup, newline delimited JSON framing, and
//! typed client side error normalization. Protocol request and response shapes
//! remain in `lilo-rm-core`.

use std::path::{Path, PathBuf};

use lilo_rm_core::{
    ErrorCode, ProtocolError, RuntimeResponse, RuntimeRpc, read_json_line, write_json_line,
};
use thiserror::Error;
use tokio::io::BufReader;
use tokio::net::UnixStream;

#[derive(Clone, Debug)]
pub struct RuntimeClient {
    socket_path: PathBuf,
}

impl RuntimeClient {
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
        }
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    pub async fn request(&self, rpc: RuntimeRpc) -> Result<RuntimeResponse, ClientError> {
        request(&self.socket_path, rpc).await
    }
}

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("rtmd unavailable at {socket_path}: {source}")]
    DaemonUnavailable {
        socket_path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("rtmd protocol error: {source}")]
    Protocol {
        #[from]
        source: ProtocolError,
    },
    #[error("rtmd returned {code}: {message}")]
    ErrorResponse { code: ErrorCode, message: String },
}

impl ClientError {
    pub const fn code(&self) -> ErrorCode {
        match self {
            Self::DaemonUnavailable { .. } => ErrorCode::RuntimeUnavailable,
            Self::Protocol { .. } => ErrorCode::ProtocolMismatch,
            Self::ErrorResponse { code, .. } => *code,
        }
    }
}

pub async fn request(
    socket_path: impl AsRef<Path>,
    rpc: RuntimeRpc,
) -> Result<RuntimeResponse, ClientError> {
    let socket_path = socket_path.as_ref();
    let stream = UnixStream::connect(socket_path).await.map_err(|source| {
        ClientError::DaemonUnavailable {
            socket_path: socket_path.to_path_buf(),
            source,
        }
    })?;
    request_on_stream(stream, rpc).await
}

async fn request_on_stream(
    stream: UnixStream,
    rpc: RuntimeRpc,
) -> Result<RuntimeResponse, ClientError> {
    let (read_half, mut write_half) = stream.into_split();
    write_json_line(&mut write_half, &rpc).await?;

    let mut reader = BufReader::new(read_half);
    match read_json_line(&mut reader).await? {
        RuntimeResponse::Error { code, message } => {
            Err(ClientError::ErrorResponse { code, message })
        }
        response => Ok(response),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::UnixListener;

    fn temp_socket_path() -> (tempfile::TempDir, PathBuf) {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let socket_path = tempdir.path().join("rtmd.sock");
        (tempdir, socket_path)
    }

    #[tokio::test]
    async fn missing_socket_reports_daemon_unavailable() {
        let (_tempdir, socket_path) = temp_socket_path();

        let error = request(&socket_path, RuntimeRpc::Version)
            .await
            .expect_err("missing socket should fail");

        match error {
            ClientError::DaemonUnavailable {
                socket_path: actual,
                ..
            } => assert_eq!(actual, socket_path),
            other => panic!("unexpected client error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn daemon_error_response_preserves_code() {
        let (_tempdir, socket_path) = temp_socket_path();
        let listener = UnixListener::bind(&socket_path).expect("bind test socket");
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept client");
            let (read_half, mut write_half) = stream.into_split();
            let mut reader = BufReader::new(read_half);
            let rpc: RuntimeRpc = read_json_line(&mut reader).await.expect("read rpc");
            assert_eq!(rpc, RuntimeRpc::Version);

            write_json_line(
                &mut write_half,
                &RuntimeResponse::error(ErrorCode::SessionNotFound, "missing session"),
            )
            .await
            .expect("write response");
        });

        let error = RuntimeClient::new(&socket_path)
            .request(RuntimeRpc::Version)
            .await
            .expect_err("daemon error response should fail");

        match error {
            ClientError::ErrorResponse { code, message } => {
                assert_eq!(code, ErrorCode::SessionNotFound);
                assert_eq!(message, "missing session");
            }
            other => panic!("unexpected client error: {other:?}"),
        }
        server.await.expect("server task");
    }
}
