use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct VersionInfo {
    pub version: String,
    pub git_sha: String,
}

pub fn version_info() -> VersionInfo {
    VersionInfo {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        git_sha: env!("RTM_GIT_SHA").to_owned(),
    }
}
