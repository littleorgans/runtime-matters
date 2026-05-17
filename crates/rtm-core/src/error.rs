use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("connection closed before a message arrived")]
    Eof,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Error)]
#[error("unsupported runtime kind: {0}")]
pub struct RuntimeKindParseError(pub String);
