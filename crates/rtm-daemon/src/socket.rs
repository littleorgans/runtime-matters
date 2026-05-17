use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

pub fn socket_path_from_env() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("RTM_SOCKET_PATH") {
        return Ok(PathBuf::from(path));
    }

    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".rtm").join("sock"))
}

pub fn display_socket_path(path: &Path) -> String {
    if std::env::var_os("RTM_SOCKET_PATH").is_some() {
        return path.display().to_string();
    }

    let Ok(default_path) = socket_path_from_env() else {
        return path.display().to_string();
    };
    if path == default_path {
        "~/.rtm/sock".to_owned()
    } else {
        path.display().to_string()
    }
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
