use std::fmt::{Display, Formatter};
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

use crate::{LaunchEnv, RuntimeKindParseError};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeKind {
    Claude,
    Codex,
    Other(String),
}

impl RuntimeKind {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Other(value) => value,
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
        if value.is_empty() {
            return Err(RuntimeKindParseError(value.to_owned()));
        }

        match value {
            "claude" => Ok(Self::Claude),
            "codex" => Ok(Self::Codex),
            other => Ok(Self::Other(other.to_owned())),
        }
    }
}

impl Serialize for RuntimeKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for RuntimeKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TmuxAddress {
    pub session: String,
    pub window: u32,
    pub pane: u32,
}

impl Display for TmuxAddress {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}:{}.{}", self.session, self.window, self.pane)
    }
}

impl FromStr for TmuxAddress {
    type Err = TmuxAddressParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (session, pane_target) = value
            .rsplit_once(':')
            .ok_or_else(|| TmuxAddressParseError(value.to_owned()))?;
        let (window, pane) = pane_target
            .split_once('.')
            .ok_or_else(|| TmuxAddressParseError(value.to_owned()))?;
        if session.is_empty() {
            return Err(TmuxAddressParseError(value.to_owned()));
        }

        Ok(Self {
            session: session.to_owned(),
            window: window
                .parse()
                .map_err(|_| TmuxAddressParseError(value.to_owned()))?,
            pane: pane
                .parse()
                .map_err(|_| TmuxAddressParseError(value.to_owned()))?,
        })
    }
}

impl Serialize for TmuxAddress {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for TmuxAddress {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("invalid tmux pane target {0}")]
pub struct TmuxAddressParseError(pub String);

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("invalid spawn target {0}; expected headless or tmux:<session>:<window>.<pane>")]
pub struct SpawnTargetParseError(pub String);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SpawnRequest {
    pub session_id: Uuid,
    pub runtime: RuntimeKind,
    #[serde(default)]
    pub env: Vec<LaunchEnv>,
    #[serde(default)]
    pub cwd: Option<std::path::PathBuf>,
    pub target: SpawnTarget,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum SpawnTarget {
    Tmux(TmuxSpawnTarget),
    Headless(HeadlessSpawnTarget),
}

impl SpawnTarget {
    pub fn tmux_address(&self) -> Option<&TmuxAddress> {
        match self {
            Self::Tmux(target) => Some(&target.address),
            Self::Headless(_) => None,
        }
    }
}

impl FromStr for SpawnTarget {
    type Err = SpawnTargetParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value == "headless" {
            return Ok(Self::Headless(HeadlessSpawnTarget {}));
        }

        let Some(address) = value.strip_prefix("tmux:") else {
            return Err(SpawnTargetParseError(value.to_owned()));
        };
        let address = address
            .parse()
            .map_err(|_| SpawnTargetParseError(value.to_owned()))?;
        Ok(Self::Tmux(TmuxSpawnTarget { address }))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TmuxSpawnTarget {
    pub address: TmuxAddress,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HeadlessSpawnTarget {}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KillRequest {
    pub session_id: Uuid,
    pub signal: RuntimeSignal,
    pub grace_secs: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NudgeRequest {
    pub session_id: Uuid,
    pub content: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ShimReady {
    pub session_id: Uuid,
    pub shim_pid: u32,
    pub runtime_pid: u32,
    pub start_time: DateTime<Utc>,
    #[serde(default)]
    pub tmux_pane: Option<TmuxAddress>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ShimLaunchRequest {
    pub session_id: Uuid,
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
    pub tmux_pane: Option<TmuxAddress>,
}

impl Lifecycle {
    pub fn forking(session_id: Uuid, runtime: RuntimeKind) -> Self {
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
        self.tmux_pane = ready.tmux_pane;
        true
    }

    pub fn mark_exited(&mut self, exit: RuntimeExit) -> bool {
        match self.state {
            LifecycleState::Forking | LifecycleState::Running => {
                self.state = LifecycleState::Exited(exit);
                true
            }
            LifecycleState::Exited(existing) => {
                if existing != exit {
                    self.state = LifecycleState::Exited(exit);
                }
                false
            }
            LifecycleState::Lost(_) => false,
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
    PidNotAlive,
    PidReuseDetected,
}

impl Display for LostEvidence {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ShimDiedBeforeReport => formatter.write_str("ShimDiedBeforeReport"),
            Self::PidNotAlive => formatter.write_str("PidNotAlive"),
            Self::PidReuseDetected => formatter.write_str("PidReuseDetected"),
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
    Lost {
        session_id: Uuid,
        evidence: LostEvidence,
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
            tmux_pane: None,
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

    #[test]
    fn tmux_pane_round_trips_as_target_string() {
        let pane: TmuxAddress = "test:0.1".parse().expect("pane");

        assert_eq!(pane.session, "test");
        assert_eq!(pane.window, 0);
        assert_eq!(pane.pane, 1);
        assert_eq!(pane.to_string(), "test:0.1");
        assert_eq!(serde_json::to_string(&pane).expect("json"), "\"test:0.1\"");

        let restored: TmuxAddress = serde_json::from_str("\"test:0.1\"").expect("restored");
        assert_eq!(restored, pane);
    }

    #[test]
    fn tmux_pane_rejects_malformed_targets() {
        for value in ["", "test", "test:window.0", "test:0", "test:0.pane"] {
            assert!(
                value.parse::<TmuxAddress>().is_err(),
                "accepted malformed pane target {value}"
            );
        }
    }

    #[test]
    fn spawn_target_parses_headless_and_tmux() {
        assert_eq!(
            "headless".parse::<SpawnTarget>().expect("headless target"),
            SpawnTarget::Headless(HeadlessSpawnTarget {})
        );
        assert_eq!(
            "tmux:test:0.1".parse::<SpawnTarget>().expect("tmux target"),
            SpawnTarget::Tmux(TmuxSpawnTarget {
                address: TmuxAddress {
                    session: "test".to_owned(),
                    window: 0,
                    pane: 1,
                },
            })
        );
    }

    #[test]
    fn spawn_target_rejects_missing_mode() {
        for value in ["", "test:0.1", "tmux:", "tmux:test", "other:test:0.1"] {
            assert!(
                value.parse::<SpawnTarget>().is_err(),
                "accepted malformed spawn target {value}"
            );
        }
    }
}
