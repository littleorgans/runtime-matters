use anyhow::{Context, Result};
use sqlx::SqlitePool;

pub async fn migrate(pool: &SqlitePool) -> Result<()> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .context("failed to run rtm-store migrations")
}
