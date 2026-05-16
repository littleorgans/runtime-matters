use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{RuntimeKind, SpawnRequest};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LaunchEnv {
    pub key: String,
    pub value: String,
}

impl LaunchEnv {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LaunchSpec {
    pub argv: Vec<String>,
    pub env: Vec<LaunchEnv>,
    pub cwd: Option<PathBuf>,
}

impl LaunchSpec {
    pub fn command(&self) -> Result<&str, LauncherError> {
        self.argv
            .first()
            .map(String::as_str)
            .ok_or(LauncherError::EmptyArgv)
    }
}

pub trait RuntimeLauncher: Sync {
    fn kind(&self) -> RuntimeKind;

    fn argv(&self, request: &SpawnRequest) -> Result<Vec<String>, LauncherError>;

    fn env(&self, request: &SpawnRequest) -> Result<Vec<LaunchEnv>, LauncherError>;

    fn cwd(&self, request: &SpawnRequest) -> Result<Option<PathBuf>, LauncherError> {
        Ok(request.cwd.clone())
    }

    fn launch_spec(&self, request: &SpawnRequest) -> Result<LaunchSpec, LauncherError> {
        let spec = LaunchSpec {
            argv: self.argv(request)?,
            env: self.env(request)?,
            cwd: self.cwd(request)?,
        };
        if spec.argv.is_empty() {
            return Err(LauncherError::EmptyArgv);
        }
        if spec.env.is_empty() {
            return Err(LauncherError::EmptyEnv {
                runtime_kind: self.kind().to_string(),
            });
        }
        Ok(spec)
    }
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum LauncherError {
    #[error("no launcher registered for runtime kind: {runtime_kind}")]
    NoLauncher { runtime_kind: String },
    #[error("launcher produced empty argv")]
    EmptyArgv,
    #[error("launcher {runtime_kind} produced empty env")]
    EmptyEnv { runtime_kind: String },
    #[error("failed to resolve launcher binary {binary}: {message}")]
    BinaryLookupFailed { binary: String, message: String },
}
