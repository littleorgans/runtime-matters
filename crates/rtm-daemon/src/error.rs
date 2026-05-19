use std::error::Error;
use std::fmt::{Display, Formatter};

use lilo_rm_core::{ErrorCode, LauncherError, RuntimeResponse, TmuxAddress};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RpcErrorContext {
    Spawn,
    Other,
}

#[derive(Debug)]
pub(crate) enum RuntimeFailure {
    ProtocolMismatch { message: String },
    SessionAlreadyExists { session_id: Uuid },
    SessionNotFound { session_id: Uuid },
    TmuxPaneDead { address: TmuxAddress },
}

impl RuntimeFailure {
    pub(crate) fn protocol_mismatch(message: impl Into<String>) -> anyhow::Error {
        Self::ProtocolMismatch {
            message: message.into(),
        }
        .into()
    }

    pub(crate) fn session_already_exists(session_id: Uuid) -> anyhow::Error {
        Self::SessionAlreadyExists { session_id }.into()
    }

    pub(crate) fn session_not_found(session_id: Uuid) -> anyhow::Error {
        Self::SessionNotFound { session_id }.into()
    }

    pub(crate) fn tmux_pane_dead(address: TmuxAddress) -> anyhow::Error {
        Self::TmuxPaneDead { address }.into()
    }

    fn code(&self) -> ErrorCode {
        match self {
            Self::ProtocolMismatch { .. } => ErrorCode::ProtocolMismatch,
            Self::SessionAlreadyExists { .. } => ErrorCode::InvalidTarget,
            Self::SessionNotFound { .. } => ErrorCode::SessionNotFound,
            Self::TmuxPaneDead { .. } => ErrorCode::TmuxPaneDead,
        }
    }
}

impl Display for RuntimeFailure {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ProtocolMismatch { message } => formatter.write_str(message),
            Self::SessionAlreadyExists { session_id } => {
                write!(formatter, "session {session_id} already exists")
            }
            Self::SessionNotFound { session_id } => {
                write!(formatter, "session {session_id} not found")
            }
            Self::TmuxPaneDead { address } => {
                write!(formatter, "tmux address {address} is not alive")
            }
        }
    }
}

impl Error for RuntimeFailure {}

pub(crate) fn protocol_error_response(error: lilo_rm_core::ProtocolError) -> RuntimeResponse {
    let code = match &error {
        lilo_rm_core::ProtocolError::Json(error) if is_invalid_target_json(error) => {
            ErrorCode::InvalidTarget
        }
        _ => ErrorCode::ProtocolMismatch,
    };
    RuntimeResponse::error(code, error.to_string())
}

pub(crate) fn rpc_error_response(
    context: RpcErrorContext,
    error: anyhow::Error,
) -> RuntimeResponse {
    let code = rpc_error_code(context, &error);
    RuntimeResponse::error(code, error.to_string())
}

fn rpc_error_code(context: RpcErrorContext, error: &anyhow::Error) -> ErrorCode {
    if let Some(failure) = find_source::<RuntimeFailure>(error) {
        return failure.code();
    }
    if let Some(error) = find_source::<LauncherError>(error) {
        return launcher_error_code(error);
    }
    if context == RpcErrorContext::Spawn {
        return ErrorCode::LaunchFailed;
    }
    ErrorCode::RuntimeUnavailable
}

fn launcher_error_code(error: &LauncherError) -> ErrorCode {
    match error {
        LauncherError::NoLauncher { .. } | LauncherError::BinaryLookupFailed { .. } => {
            ErrorCode::RuntimeUnavailable
        }
        LauncherError::EmptyArgv
        | LauncherError::EmptyEnv { .. }
        | LauncherError::EmptyShellArgv => ErrorCode::LaunchFailed,
    }
}

fn is_invalid_target_json(error: &serde_json::Error) -> bool {
    let message = error.to_string();
    message.contains("invalid spawn target") || message.contains("invalid tmux pane target")
}

fn find_source<T>(error: &anyhow::Error) -> Option<&T>
where
    T: Error + 'static,
{
    error.chain().find_map(|source| source.downcast_ref::<T>())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use serde_json::json;

    #[test]
    fn runtime_failures_map_to_stable_error_codes() {
        let session_id = Uuid::parse_str("018f6e28-0000-7000-8000-000000000001").unwrap();
        let address = "rtm:0.1".parse().unwrap();
        let cases = [
            (
                RuntimeFailure::session_not_found(session_id),
                ErrorCode::SessionNotFound,
            ),
            (
                RuntimeFailure::tmux_pane_dead(address),
                ErrorCode::TmuxPaneDead,
            ),
            (
                RuntimeFailure::session_already_exists(session_id),
                ErrorCode::InvalidTarget,
            ),
            (
                RuntimeFailure::protocol_mismatch("bad shim state"),
                ErrorCode::ProtocolMismatch,
            ),
        ];

        for (error, expected) in cases {
            let RuntimeResponse::Error(payload) = rpc_error_response(RpcErrorContext::Other, error)
            else {
                panic!("expected error response");
            };
            assert_eq!(payload.code, expected);
        }
    }

    #[test]
    fn spawn_errors_default_to_launch_failed() {
        let RuntimeResponse::Error(payload) =
            rpc_error_response(RpcErrorContext::Spawn, anyhow!("boom"))
        else {
            panic!("expected error response");
        };

        assert_eq!(payload.code, ErrorCode::LaunchFailed);
        assert_eq!(payload.message, "boom");
    }

    #[test]
    fn launcher_errors_map_to_specific_availability_codes() {
        let cases = [
            (
                LauncherError::NoLauncher {
                    runtime_kind: "missing".to_owned(),
                },
                ErrorCode::RuntimeUnavailable,
            ),
            (
                LauncherError::EmptyEnv {
                    runtime_kind: "claude".to_owned(),
                },
                ErrorCode::LaunchFailed,
            ),
            (LauncherError::EmptyArgv, ErrorCode::LaunchFailed),
            (
                LauncherError::BinaryLookupFailed {
                    binary: "claude".to_owned(),
                    message: "which failed".to_owned(),
                },
                ErrorCode::RuntimeUnavailable,
            ),
        ];

        for (error, expected) in cases {
            let RuntimeResponse::Error(payload) =
                rpc_error_response(RpcErrorContext::Other, error.into())
            else {
                panic!("expected error response");
            };
            assert_eq!(payload.code, expected);
        }
    }

    #[test]
    fn invalid_target_decode_maps_to_invalid_target() {
        let error = serde_json::from_value::<lilo_rm_core::SpawnRequest>(json!({
            "session_id": "018f6e28-0000-7000-8000-000000000001",
            "runtime": "claude",
            "env": [],
            "cwd": "/tmp/rtm",
            "target": {
                "type": "tmux",
                "payload": {
                    "address": "not-a-pane"
                }
            }
        }))
        .expect_err("invalid tmux target");

        let RuntimeResponse::Error(payload) =
            protocol_error_response(lilo_rm_core::ProtocolError::Json(error))
        else {
            panic!("expected error response");
        };
        assert_eq!(payload.code, ErrorCode::InvalidTarget);
    }
}
