use std::path::PathBuf;

use anyhow::{Context, Result};

#[derive(Clone, Debug)]
pub struct StoreConfig {
    pub db_path: PathBuf,
}

impl StoreConfig {
    pub fn from_env() -> Result<Self> {
        let db_path = match std::env::var_os("RTM_DB_PATH") {
            Some(path) => PathBuf::from(path),
            None => default_db_path()?,
        };
        Ok(Self { db_path })
    }
}

fn default_db_path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is required for default rtm db path")?;
    Ok(PathBuf::from(home).join(".rtm").join("db.sqlite"))
}
