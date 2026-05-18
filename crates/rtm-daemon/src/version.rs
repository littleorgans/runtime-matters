use lilo_rm_core::VersionInfo;

pub(crate) fn runtime_version_info() -> VersionInfo {
    VersionInfo {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        git_sha: lilo_rm_core::version_info().git_sha,
    }
}
