use std::fmt::{Display, Formatter};
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::RuntimeKindParseError;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    Claude,
}

impl RuntimeKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
        }
    }
}

impl Display for RuntimeKind {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for RuntimeKind {
    type Err = RuntimeKindParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "claude" => Ok(Self::Claude),
            other => Err(RuntimeKindParseError(other.to_owned())),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleState {
    Forking,
    Running,
    Exited(RuntimeExit),
    Lost(LostEvidence),
}

impl Display for LifecycleState {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Forking => formatter.write_str("Forking"),
            Self::Running => formatter.write_str("Running"),
            Self::Exited(exit) => write!(formatter, "Exited({exit})"),
            Self::Lost(evidence) => write!(formatter, "Lost({evidence})"),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeSignal {
    Hup,
    Int,
    Term,
    Kill,
}

impl RuntimeSignal {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hup => "HUP",
            Self::Int => "INT",
            Self::Term => "TERM",
            Self::Kill => "KILL",
        }
    }
}

impl Display for RuntimeSignal {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for RuntimeSignal {
    type Err = RuntimeSignalParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value
            .trim_start_matches("SIG")
            .to_ascii_uppercase()
            .as_str()
        {
            "HUP" => Ok(Self::Hup),
            "INT" => Ok(Self::Int),
            "TERM" => Ok(Self::Term),
            "KILL" => Ok(Self::Kill),
            other => Err(RuntimeSignalParseError(other.to_owned())),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("unsupported signal {0}")]
pub struct RuntimeSignalParseError(pub String);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SpawnRequest {
    pub session_id: Uuid,
    pub runtime: RuntimeKind,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KillRequest {
    pub session_id: Uuid,
    pub signal: RuntimeSignal,
    pub grace_secs: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ShimReady {
    pub session_id: Uuid,
    pub shim_pid: u32,
    pub runtime_pid: u32,
    pub start_time: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ShimExit {
    pub session_id: Uuid,
    pub exit: RuntimeExit,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Lifecycle {
    pub session_id: Uuid,
    pub runtime: RuntimeKind,
    pub state: LifecycleState,
    pub shim_pid: Option<u32>,
    pub runtime_pid: Option<u32>,
    pub start_time: Option<DateTime<Utc>>,
    pub tmux_pane: Option<String>,
}

impl Lifecycle {
    pub const fn forking(session_id: Uuid, runtime: RuntimeKind) -> Self {
        Self {
            session_id,
            runtime,
            state: LifecycleState::Forking,
            shim_pid: None,
            runtime_pid: None,
            start_time: None,
            tmux_pane: None,
        }
    }

    pub fn mark_running(&mut self, ready: ShimReady) -> bool {
        if self.state != LifecycleState::Forking {
            return false;
        }
        self.state = LifecycleState::Running;
        self.shim_pid = Some(ready.shim_pid);
        self.runtime_pid = Some(ready.runtime_pid);
        self.start_time = Some(ready.start_time);
        true
    }

    pub fn mark_exited(&mut self, exit: RuntimeExit) -> bool {
        match self.state {
            LifecycleState::Forking | LifecycleState::Running | LifecycleState::Lost(_) => {
                self.state = LifecycleState::Exited(exit);
                true
            }
            LifecycleState::Exited(existing) => {
                if existing != exit {
                    self.state = LifecycleState::Exited(exit);
                }
                false
            }
        }
    }

    pub fn mark_lost(&mut self, evidence: LostEvidence) -> bool {
        match self.state {
            LifecycleState::Forking | LifecycleState::Running => {
                self.state = LifecycleState::Lost(evidence);
                true
            }
            LifecycleState::Exited(_) | LifecycleState::Lost(_) => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RuntimeExit {
    pub code: Option<i32>,
    pub signal: Option<i32>,
}

impl RuntimeExit {
    pub const fn new(code: Option<i32>, signal: Option<i32>) -> Self {
        Self { code, signal }
    }
}

impl Display for RuntimeExit {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match (self.code, self.signal) {
            (Some(code), _) => write!(formatter, "code={code}"),
            (None, Some(signal)) => write!(formatter, "signal={signal}"),
            (None, None) => formatter.write_str("unknown"),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LostEvidence {
    ShimDiedBeforeReport,
}

impl Display for LostEvidence {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ShimDiedBeforeReport => formatter.write_str("ShimDiedBeforeReport"),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminationEvidence {
    ShimExit,
    KqueueExit,
    Lost(LostEvidence),
}

impl Display for TerminationEvidence {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ShimExit => formatter.write_str("ShimExit"),
            Self::KqueueExit => formatter.write_str("KqueueExit"),
            Self::Lost(evidence) => write!(formatter, "Lost({evidence})"),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum RuntimeEvent {
    Running {
        session_id: Uuid,
        runtime_pid: u32,
        start_time: DateTime<Utc>,
    },
    Terminated {
        session_id: Uuid,
        exit_code: Option<i32>,
        signal: Option<i32>,
        evidence: TerminationEvidence,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready(session_id: Uuid) -> ShimReady {
        ShimReady {
            session_id,
            shim_pid: 100,
            runtime_pid: 200,
            start_time: Utc::now(),
        }
    }

    #[test]
    fn lifecycle_transitions_from_forking_to_running_to_exited() {
        let session_id = Uuid::now_v7();
        let mut lifecycle = Lifecycle::forking(session_id, RuntimeKind::Claude);

        assert_eq!(lifecycle.state, LifecycleState::Forking);
        assert!(lifecycle.mark_running(ready(session_id)));
        assert_eq!(lifecycle.state, LifecycleState::Running);
        assert!(lifecycle.mark_exited(RuntimeExit::new(Some(0), None)));
        assert_eq!(
            lifecycle.state,
            LifecycleState::Exited(RuntimeExit::new(Some(0), None))
        );
    }

    #[test]
    fn lifecycle_transitions_from_forking_to_lost() {
        let session_id = Uuid::now_v7();
        let mut lifecycle = Lifecycle::forking(session_id, RuntimeKind::Claude);

        assert!(lifecycle.mark_lost(LostEvidence::ShimDiedBeforeReport));
        assert_eq!(
            lifecycle.state,
            LifecycleState::Lost(LostEvidence::ShimDiedBeforeReport)
        );
    }

    #[test]
    fn lifecycle_transitions_from_running_to_lost() {
        let session_id = Uuid::now_v7();
        let mut lifecycle = Lifecycle::forking(session_id, RuntimeKind::Claude);
        assert!(lifecycle.mark_running(ready(session_id)));

        assert!(lifecycle.mark_lost(LostEvidence::ShimDiedBeforeReport));
        assert_eq!(
            lifecycle.state,
            LifecycleState::Lost(LostEvidence::ShimDiedBeforeReport)
        );
    }
}
