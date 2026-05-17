use std::path::PathBuf;
use std::str::FromStr;

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use rtm_core::{Lifecycle, LifecycleState, LostEvidence, RuntimeExit, RuntimeKind, TmuxPane};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Executor, SqlitePool};
use uuid::Uuid;

use crate::{StoreConfig, schema};

#[derive(Clone)]
pub struct LifecycleStore {
    pool: SqlitePool,
}

impl LifecycleStore {
    pub async fn open(config: StoreConfig) -> Result<Self> {
        if let Some(parent) = config.db_path.parent() {
            tokio::fs::create_dir_all(parent).await.with_context(|| {
                format!("failed to create rtm db directory {}", parent.display())
            })?;
        }
        let options = SqliteConnectOptions::new()
            .filename(&config.db_path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal);
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await
            .with_context(|| format!("failed to open sqlite db {}", config.db_path.display()))?;
        schema::migrate(&pool).await?;
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn insert_forking(&self, lifecycle: &Lifecycle) -> Result<()> {
        if lifecycle.state != LifecycleState::Forking {
            bail!("insert_forking requires Forking lifecycle state");
        }
        let encoded = EncodedLifecycle::from_lifecycle(lifecycle)?;
        sqlx::query(
            r#"
            INSERT INTO lifecycle (
                session_id, runtime, state, shim_pid, runtime_pid, start_time,
                tmux_pane, exit_code, exit_signal, lost_evidence, spawned_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(encoded.session_id)
        .bind(encoded.runtime)
        .bind(encoded.state)
        .bind(encoded.shim_pid)
        .bind(encoded.runtime_pid)
        .bind(encoded.start_time)
        .bind(encoded.tmux_pane)
        .bind(encoded.exit_code)
        .bind(encoded.exit_signal)
        .bind(encoded.lost_evidence)
        .bind(encoded.now.clone())
        .bind(encoded.now)
        .execute(&self.pool)
        .await
        .with_context(|| format!("failed to insert lifecycle {}", lifecycle.session_id))?;
        Ok(())
    }

    pub async fn update_lifecycle(&self, lifecycle: &Lifecycle) -> Result<()> {
        let encoded = EncodedLifecycle::from_lifecycle(lifecycle)?;
        let result = sqlx::query(
            r#"
            UPDATE lifecycle
            SET runtime = ?,
                state = ?,
                shim_pid = ?,
                runtime_pid = ?,
                start_time = ?,
                tmux_pane = ?,
                exit_code = ?,
                exit_signal = ?,
                lost_evidence = ?,
                updated_at = ?
            WHERE session_id = ?
            "#,
        )
        .bind(encoded.runtime)
        .bind(encoded.state)
        .bind(encoded.shim_pid)
        .bind(encoded.runtime_pid)
        .bind(encoded.start_time)
        .bind(encoded.tmux_pane)
        .bind(encoded.exit_code)
        .bind(encoded.exit_signal)
        .bind(encoded.lost_evidence)
        .bind(encoded.now)
        .bind(encoded.session_id)
        .execute(&self.pool)
        .await
        .with_context(|| format!("failed to update lifecycle {}", lifecycle.session_id))?;
        if result.rows_affected() == 0 {
            bail!("session {} not found", lifecycle.session_id);
        }
        Ok(())
    }

    pub async fn delete(&self, session_id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM lifecycle WHERE session_id = ?")
            .bind(session_id.to_string())
            .execute(&self.pool)
            .await
            .with_context(|| format!("failed to delete lifecycle {session_id}"))?;
        Ok(())
    }

    pub async fn get(&self, session_id: Uuid) -> Result<Option<Lifecycle>> {
        let row = sqlx::query_as::<_, LifecycleRow>(
            r#"
            SELECT session_id, runtime, state, shim_pid, runtime_pid, start_time,
                   tmux_pane, exit_code, exit_signal, lost_evidence
            FROM lifecycle
            WHERE session_id = ?
            "#,
        )
        .bind(session_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .with_context(|| format!("failed to fetch lifecycle {session_id}"))?;
        row.map(TryInto::try_into).transpose()
    }

    pub async fn list(&self, session_id: Option<Uuid>) -> Result<Vec<Lifecycle>> {
        let rows = match session_id {
            Some(id) => {
                sqlx::query_as::<_, LifecycleRow>(
                    r#"
                    SELECT session_id, runtime, state, shim_pid, runtime_pid, start_time,
                           tmux_pane, exit_code, exit_signal, lost_evidence
                    FROM lifecycle
                    WHERE session_id = ?
                    ORDER BY session_id
                    "#,
                )
                .bind(id.to_string())
                .fetch_all(&self.pool)
                .await
            }
            None => {
                sqlx::query_as::<_, LifecycleRow>(
                    r#"
                    SELECT session_id, runtime, state, shim_pid, runtime_pid, start_time,
                           tmux_pane, exit_code, exit_signal, lost_evidence
                    FROM lifecycle
                    ORDER BY session_id
                    "#,
                )
                .fetch_all(&self.pool)
                .await
            }
        }
        .context("failed to list lifecycles")?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    pub async fn running(&self) -> Result<Vec<Lifecycle>> {
        let rows = sqlx::query_as::<_, LifecycleRow>(
            r#"
            SELECT session_id, runtime, state, shim_pid, runtime_pid, start_time,
                   tmux_pane, exit_code, exit_signal, lost_evidence
            FROM lifecycle
            WHERE state = 'Running'
            ORDER BY spawned_at
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .context("failed to list running lifecycles")?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    pub async fn reset(&self) -> Result<()> {
        self.pool
            .execute("DELETE FROM lifecycle")
            .await
            .context("failed to reset lifecycle table")?;
        Ok(())
    }

    pub async fn path_open(path: PathBuf) -> Result<Self> {
        Self::open(StoreConfig { db_path: path }).await
    }
}

#[derive(sqlx::FromRow)]
struct LifecycleRow {
    session_id: String,
    runtime: String,
    state: String,
    shim_pid: Option<i64>,
    runtime_pid: Option<i64>,
    start_time: Option<String>,
    tmux_pane: Option<String>,
    exit_code: Option<i64>,
    exit_signal: Option<i64>,
    lost_evidence: Option<String>,
}

struct EncodedLifecycle {
    session_id: String,
    runtime: String,
    state: &'static str,
    shim_pid: Option<i64>,
    runtime_pid: Option<i64>,
    start_time: Option<String>,
    tmux_pane: Option<String>,
    exit_code: Option<i64>,
    exit_signal: Option<i64>,
    lost_evidence: Option<&'static str>,
    now: String,
}

impl EncodedLifecycle {
    fn from_lifecycle(lifecycle: &Lifecycle) -> Result<Self> {
        let (state, exit_code, exit_signal, lost_evidence) = encode_state(&lifecycle.state);
        Ok(Self {
            session_id: lifecycle.session_id.to_string(),
            runtime: lifecycle.runtime.to_string(),
            state,
            shim_pid: lifecycle.shim_pid.map(i64::from),
            runtime_pid: lifecycle.runtime_pid.map(i64::from),
            start_time: lifecycle.start_time.map(|time| time.to_rfc3339()),
            tmux_pane: encode_tmux_pane(lifecycle.tmux_pane.as_ref())?,
            exit_code: exit_code.map(i64::from),
            exit_signal: exit_signal.map(i64::from),
            lost_evidence,
            now: Utc::now().to_rfc3339(),
        })
    }
}

impl TryFrom<LifecycleRow> for Lifecycle {
    type Error = anyhow::Error;

    fn try_from(row: LifecycleRow) -> Result<Self> {
        Ok(Self {
            session_id: Uuid::parse_str(&row.session_id)?,
            runtime: RuntimeKind::from_str(&row.runtime)?,
            state: decode_state(&row)?,
            shim_pid: decode_u32(row.shim_pid, "shim_pid")?,
            runtime_pid: decode_u32(row.runtime_pid, "runtime_pid")?,
            start_time: row.start_time.map(|time| parse_time(&time)).transpose()?,
            tmux_pane: decode_tmux_pane(row.tmux_pane)?,
        })
    }
}

fn encode_tmux_pane(tmux_pane: Option<&TmuxPane>) -> Result<Option<String>> {
    Ok(tmux_pane.map(serde_json::to_string).transpose()?)
}

fn decode_tmux_pane(tmux_pane: Option<String>) -> Result<Option<TmuxPane>> {
    tmux_pane
        .map(|value| -> Result<TmuxPane> {
            if let Ok(pane) = serde_json::from_str::<TmuxPane>(&value) {
                return Ok(pane);
            }
            Ok(value.parse()?)
        })
        .transpose()
        .context("invalid stored tmux pane")
}

fn encode_state(
    state: &LifecycleState,
) -> (&'static str, Option<i32>, Option<i32>, Option<&'static str>) {
    match state {
        LifecycleState::Forking => ("Forking", None, None, None),
        LifecycleState::Running => ("Running", None, None, None),
        LifecycleState::Exited(exit) => ("Exited", exit.code, exit.signal, None),
        LifecycleState::Lost(evidence) => ("Lost", None, None, Some(encode_lost(*evidence))),
    }
}

fn decode_state(row: &LifecycleRow) -> Result<LifecycleState> {
    match row.state.as_str() {
        "Forking" => Ok(LifecycleState::Forking),
        "Running" => Ok(LifecycleState::Running),
        "Exited" => Ok(LifecycleState::Exited(RuntimeExit::new(
            decode_i32(row.exit_code, "exit_code")?,
            decode_i32(row.exit_signal, "exit_signal")?,
        ))),
        "Lost" => Ok(LifecycleState::Lost(decode_lost(
            row.lost_evidence.as_deref(),
        )?)),
        state => Err(anyhow!("unknown lifecycle state {state}")),
    }
}

fn encode_lost(evidence: LostEvidence) -> &'static str {
    match evidence {
        LostEvidence::ShimDiedBeforeReport => "ShimDiedBeforeReport",
        LostEvidence::PidNotAlive => "PidNotAlive",
        LostEvidence::PidReuseDetected => "PidReuseDetected",
    }
}

fn decode_lost(value: Option<&str>) -> Result<LostEvidence> {
    match value {
        Some("ShimDiedBeforeReport") => Ok(LostEvidence::ShimDiedBeforeReport),
        Some("PidNotAlive") => Ok(LostEvidence::PidNotAlive),
        Some("PidReuseDetected") => Ok(LostEvidence::PidReuseDetected),
        Some(other) => Err(anyhow!("unknown lost evidence {other}")),
        None => Err(anyhow!("lost lifecycle missing evidence")),
    }
}

fn decode_u32(value: Option<i64>, field: &'static str) -> Result<Option<u32>> {
    value
        .map(|inner| u32::try_from(inner).with_context(|| format!("{field} out of range")))
        .transpose()
}

fn decode_i32(value: Option<i64>, field: &'static str) -> Result<Option<i32>> {
    value
        .map(|inner| i32::try_from(inner).with_context(|| format!("{field} out of range")))
        .transpose()
}

fn parse_time(value: &str) -> Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(value)?.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtm_core::ShimReady;
    use tempfile::TempDir;

    #[tokio::test]
    async fn persists_lifecycle_transitions() {
        let temp = TempDir::new().expect("temp dir");
        let store = LifecycleStore::path_open(temp.path().join("rtm.sqlite"))
            .await
            .expect("store");
        let session_id = Uuid::now_v7();
        let mut lifecycle = Lifecycle::forking(session_id, RuntimeKind::Claude);

        store.insert_forking(&lifecycle).await.expect("insert");
        lifecycle.state = LifecycleState::Lost(LostEvidence::PidNotAlive);
        store.update_lifecycle(&lifecycle).await.expect("update");

        let restored = store.get(session_id).await.expect("get").expect("row");
        assert_eq!(
            restored.state,
            LifecycleState::Lost(LostEvidence::PidNotAlive)
        );
        assert_eq!(store.running().await.expect("running").len(), 0);
    }

    #[tokio::test]
    async fn tmux_pane_round_trips_through_sqlite() {
        let temp = TempDir::new().expect("temp dir");
        let store = LifecycleStore::path_open(temp.path().join("rtm.sqlite"))
            .await
            .expect("store");
        let session_id = Uuid::now_v7();
        let mut lifecycle = Lifecycle::forking(session_id, RuntimeKind::Claude);
        lifecycle.mark_running(ShimReady {
            session_id,
            shim_pid: 10,
            runtime_pid: 20,
            start_time: Utc::now(),
            tmux_pane: Some("test:0.1".parse().expect("tmux pane")),
        });

        store
            .insert_forking(&Lifecycle::forking(session_id, RuntimeKind::Claude))
            .await
            .expect("insert");
        store.update_lifecycle(&lifecycle).await.expect("update");

        let restored = store.get(session_id).await.expect("get").expect("row");
        assert_eq!(restored.tmux_pane, lifecycle.tmux_pane);
    }

    #[tokio::test]
    async fn migration_is_idempotent() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("rtm.sqlite");

        LifecycleStore::path_open(path.clone())
            .await
            .expect("first open");
        LifecycleStore::path_open(path).await.expect("second open");
    }
}
