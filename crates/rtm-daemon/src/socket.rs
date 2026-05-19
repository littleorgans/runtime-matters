use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

pub fn socket_path_from_env() -> Result<PathBuf> {
    default_socket_path(
        std::env::var_os("RTM_SOCKET_PATH"),
        std::env::var_os("XDG_RUNTIME_DIR"),
        std::env::var_os("HOME"),
    )
}

pub fn display_socket_path(path: &Path) -> String {
    if std::env::var_os("RTM_SOCKET_PATH").is_some() {
        return path.display().to_string();
    }

    let Ok(default_path) = socket_path_from_env() else {
        return path.display().to_string();
    };
    if path == default_path {
        default_socket_path_label(std::env::var_os("XDG_RUNTIME_DIR"))
    } else {
        path.display().to_string()
    }
}

fn default_socket_path(
    override_path: Option<std::ffi::OsString>,
    xdg_runtime_dir: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> Result<PathBuf> {
    if let Some(path) = override_path {
        return Ok(PathBuf::from(path));
    }

    platform_default_socket_path(xdg_runtime_dir, home)
}

#[cfg(target_os = "linux")]
fn non_empty_path(value: Option<std::ffi::OsString>) -> Option<PathBuf> {
    value.filter(|value| !value.is_empty()).map(PathBuf::from)
}

#[cfg(target_os = "linux")]
fn platform_default_socket_path(
    xdg_runtime_dir: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> Result<PathBuf> {
    if let Some(runtime_dir) = non_empty_path(xdg_runtime_dir) {
        return Ok(runtime_dir.join("rtm").join("sock"));
    }

    home_default_socket_path(home)
}

#[cfg(not(target_os = "linux"))]
fn platform_default_socket_path(
    _xdg_runtime_dir: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> Result<PathBuf> {
    home_default_socket_path(home)
}

fn home_default_socket_path(home: Option<std::ffi::OsString>) -> Result<PathBuf> {
    let home = home.context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".rtm").join("sock"))
}

fn default_socket_path_label(xdg_runtime_dir: Option<std::ffi::OsString>) -> String {
    platform_default_socket_path_label(xdg_runtime_dir)
}

#[cfg(target_os = "linux")]
fn platform_default_socket_path_label(xdg_runtime_dir: Option<std::ffi::OsString>) -> String {
    if non_empty_path(xdg_runtime_dir).is_some() {
        return "$XDG_RUNTIME_DIR/rtm/sock".to_owned();
    }

    "~/.rtm/sock".to_owned()
}

#[cfg(not(target_os = "linux"))]
fn platform_default_socket_path_label(_xdg_runtime_dir: Option<std::ffi::OsString>) -> String {
    "~/.rtm/sock".to_owned()
}

pub fn prepare_socket(path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("socket path {} has no parent", path.display()))?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("failed to create {}", parent.display()))?;

    remove_socket_file(path)?;
    Ok(())
}

pub fn remove_socket_file(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => bail!("failed to remove {}: {error}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::{default_socket_path, default_socket_path_label};

    #[test]
    fn explicit_socket_path_wins() {
        let path = default_socket_path(
            Some(OsString::from("/tmp/custom.sock")),
            Some(OsString::from("/run/user/1000")),
            Some(OsString::from("/home/alice")),
        )
        .expect("socket path");

        assert_eq!(path, PathBuf::from("/tmp/custom.sock"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_uses_xdg_runtime_dir_when_available() {
        let path = default_socket_path(
            None,
            Some(OsString::from("/run/user/1000")),
            Some(OsString::from("/home/alice")),
        )
        .expect("socket path");

        assert_eq!(path, PathBuf::from("/run/user/1000/rtm/sock"));
    }

    #[test]
    fn falls_back_to_home_runtime_dir() {
        let path = default_socket_path(None, None, Some(OsString::from("/home/alice")))
            .expect("socket path");

        assert_eq!(path, PathBuf::from("/home/alice/.rtm/sock"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_default_display_names_xdg_runtime_dir() {
        let label = default_socket_path_label(Some(OsString::from("/run/user/1000")));

        assert_eq!(label, "$XDG_RUNTIME_DIR/rtm/sock");
    }

    #[test]
    fn home_fallback_display_is_stable() {
        let label = default_socket_path_label(None);

        assert_eq!(label, "~/.rtm/sock");
    }
}
