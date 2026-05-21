use std::fmt::{Display, Formatter};
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

use crate::{IsolationPolicy, LaunchEnv, RuntimeKind, RuntimeSignal, ShellResume};

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
    pub isolation: IsolationPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(default)]
    pub env: Vec<LaunchEnv>,
    pub cwd: std::path::PathBuf,
    pub target: SpawnTarget,
    #[serde(default, skip_serializing_if = "is_false")]
    pub force: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell_resume: Option<ShellResume>,
}

fn is_false(value: &bool) -> bool {
    !*value
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

#[cfg(test)]
mod tests {
    use super::*;

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
