use std::path::PathBuf;
use std::str::FromStr;

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use lilo_rm_core::{
    IsolationPolicy, Lifecycle, LifecycleCounts, LifecycleState, LostEvidence, MigrationState,
    RecentLostEvent, RuntimeExit, RuntimeKind, StatusFilter, TmuxAddress,
};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Executor, QueryBuilder, Sqlite, SqlitePool};
use uuid::Uuid;

use crate::{StoreConfig, schema};

const LAST_PROBE_SWEEP_KEY: &str = "last_probe_sweep_at";

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
                session_id, runtime, isolation, state, shim_pid, runtime_pid, start_time,
                tmux_pane, exit_code, exit_signal, lost_evidence, spawned_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(encoded.session_id)
        .bind(encoded.runtime)
        .bind(encoded.isolation)
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
                isolation = ?,
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
        .bind(encoded.isolation)
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
            SELECT session_id, runtime, isolation, state, shim_pid, runtime_pid, start_time,
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

    pub async fn list(&self, filter: &StatusFilter) -> Result<Vec<Lifecycle>> {
        let session_ids = filter.requested_session_ids();
        let mut query = QueryBuilder::<Sqlite>::new(
            r#"
            SELECT session_id, runtime, isolation, state, shim_pid, runtime_pid, start_time,
                   tmux_pane, exit_code, exit_signal, lost_evidence
            FROM lifecycle
            "#,
        );
        let mut has_where = false;
        if !session_ids.is_empty() {
            push_where(&mut query, &mut has_where);
            query.push("session_id IN (");
            {
                let mut separated = query.separated(", ");
                for session_id in session_ids {
                    separated.push_bind(session_id.to_string());
                }
            }
            query.push(")");
        }
        if let Some(runtime) = &filter.runtime {
            push_where(&mut query, &mut has_where);
            query.push("runtime = ");
            query.push_bind(runtime);
        }
        if let Some(state) = &filter.state {
            push_where(&mut query, &mut has_where);
            query.push("LOWER(state) = LOWER(");
            query.push_bind(state);
            query.push(")");
        }
        if let Some(updated_since) = &filter.updated_since {
            push_where(&mut query, &mut has_where);
            query.push("updated_at >= ");
            query.push_bind(updated_since.to_rfc3339());
        }
        query.push(" ORDER BY session_id");

        let rows = query
            .build_query_as::<LifecycleRow>()
            .fetch_all(&self.pool)
            .await
            .context("failed to list lifecycles")?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    pub async fn running(&self) -> Result<Vec<Lifecycle>> {
        let rows = sqlx::query_as::<_, LifecycleRow>(
            r#"
            SELECT session_id, runtime, isolation, state, shim_pid, runtime_pid, start_time,
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

    pub async fn running_tmux_occupant(
        &self,
        tmux_pane: &lilo_rm_core::TmuxAddress,
    ) -> Result<Option<Lifecycle>> {
        let row = sqlx::query_as::<_, LifecycleRow>(
            r#"
            SELECT session_id, runtime, isolation, state, shim_pid, runtime_pid, start_time,
                   tmux_pane, exit_code, exit_signal, lost_evidence
            FROM lifecycle
            WHERE state = 'Running' AND tmux_pane = ?
            ORDER BY spawned_at
            LIMIT 1
            "#,
        )
        .bind(encode_tmux_pane(Some(tmux_pane))?)
        .fetch_optional(&self.pool)
        .await
        .context("failed to fetch running tmux pane occupant")?;
        row.map(TryInto::try_into).transpose()
    }

    pub async fn lifecycle_counts(&self) -> Result<LifecycleCounts> {
        let rows = sqlx::query_as::<_, StateCountRow>(
            r#"
            SELECT state, COUNT(*) AS count
            FROM lifecycle
            GROUP BY state
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .context("failed to count lifecycle states")?;

        let mut counts = LifecycleCounts::default();
        for row in rows {
            let count = u64::try_from(row.count).context("lifecycle count out of range")?;
            match row.state.as_str() {
                "Forking" => counts.forking = count,
                "Running" => counts.running = count,
                "Exited" => counts.exited = count,
                "Lost" => counts.lost = count,
                state => bail!("unknown lifecycle state {state}"),
            }
        }
        Ok(counts)
    }

    pub async fn recent_lost_since(&self, since: DateTime<Utc>) -> Result<Vec<RecentLostEvent>> {
        let rows = sqlx::query_as::<_, RecentLostRow>(
            r#"
            SELECT session_id, lost_evidence, updated_at
            FROM lifecycle
            WHERE state = 'Lost' AND updated_at >= ?
            ORDER BY updated_at DESC, session_id
            "#,
        )
        .bind(since.to_rfc3339())
        .fetch_all(&self.pool)
        .await
        .context("failed to list recent lost lifecycles")?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    pub async fn record_probe_sweep(&self, swept_at: DateTime<Utc>) -> Result<()> {
        let value = swept_at.to_rfc3339();
        sqlx::query(
            r#"
            INSERT INTO rtm_metadata (key, value, updated_at)
            VALUES (?, ?, ?)
            ON CONFLICT(key) DO UPDATE SET
                value = excluded.value,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(LAST_PROBE_SWEEP_KEY)
        .bind(value.clone())
        .bind(value)
        .execute(&self.pool)
        .await
        .context("failed to record last probe sweep")?;
        Ok(())
    }

    pub async fn last_probe_sweep(&self) -> Result<Option<DateTime<Utc>>> {
        let value = sqlx::query_scalar::<_, String>(
            r#"
            SELECT value
            FROM rtm_metadata
            WHERE key = ?
            "#,
        )
        .bind(LAST_PROBE_SWEEP_KEY)
        .fetch_optional(&self.pool)
        .await
        .context("failed to read last probe sweep")?;
        value.map(|time| parse_time(&time)).transpose()
    }

    pub async fn migration_state(&self) -> Result<MigrationState> {
        let known = schema::known_migrations();
        let applied_versions = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT version
            FROM _sqlx_migrations
            WHERE success = 1
            ORDER BY version
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .context("failed to read applied migrations")?;

        let mut applied_descriptions = Vec::new();
        let mut pending_descriptions = Vec::new();
        for migration in &known {
            if applied_versions.contains(&migration.version) {
                applied_descriptions.push(migration.description.clone());
            } else {
                pending_descriptions.push(migration.description.clone());
            }
        }
        Ok(MigrationState {
            applied: applied_descriptions.len(),
            total: known.len(),
            applied_descriptions,
            pending_descriptions,
        })
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
    isolation: String,
    state: String,
    shim_pid: Option<i64>,
    runtime_pid: Option<i64>,
    start_time: Option<String>,
    tmux_pane: Option<String>,
    exit_code: Option<i64>,
    exit_signal: Option<i64>,
    lost_evidence: Option<String>,
}

#[derive(sqlx::FromRow)]
struct StateCountRow {
    state: String,
    count: i64,
}

#[derive(sqlx::FromRow)]
struct RecentLostRow {
    session_id: String,
    lost_evidence: Option<String>,
    updated_at: String,
}

struct EncodedLifecycle {
    session_id: String,
    runtime: String,
    isolation: String,
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

type EncodedState = (&'static str, Option<i32>, Option<i32>, Option<&'static str>);

impl EncodedLifecycle {
    fn from_lifecycle(lifecycle: &Lifecycle) -> Result<Self> {
        let (state, exit_code, exit_signal, lost_evidence) = encode_state(&lifecycle.state)?;
        Ok(Self {
            session_id: lifecycle.session_id.to_string(),
            runtime: lifecycle.runtime.to_string(),
            isolation: lifecycle.isolation.to_string(),
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
            isolation: IsolationPolicy::from_str(&row.isolation)?,
            state: decode_state(&row)?,
            shim_pid: decode_u32(row.shim_pid, "shim_pid")?,
            runtime_pid: decode_u32(row.runtime_pid, "runtime_pid")?,
            start_time: row.start_time.map(|time| parse_time(&time)).transpose()?,
            tmux_pane: decode_tmux_pane(row.tmux_pane)?,
            log_availability: None,
        })
    }
}

impl TryFrom<RecentLostRow> for RecentLostEvent {
    type Error = anyhow::Error;

    fn try_from(row: RecentLostRow) -> Result<Self> {
        Ok(Self {
            session_id: Uuid::parse_str(&row.session_id)?,
            evidence: decode_lost(row.lost_evidence.as_deref())?,
            occurred_at: parse_time(&row.updated_at)?,
        })
    }
}

fn encode_tmux_pane(tmux_pane: Option<&TmuxAddress>) -> Result<Option<String>> {
    Ok(tmux_pane.map(serde_json::to_string).transpose()?)
}

fn decode_tmux_pane(tmux_pane: Option<String>) -> Result<Option<TmuxAddress>> {
    tmux_pane
        .map(|value| -> Result<TmuxAddress> {
            if let Ok(pane) = serde_json::from_str::<TmuxAddress>(&value) {
                return Ok(pane);
            }
            Ok(value.parse()?)
        })
        .transpose()
        .context("invalid stored tmux pane")
}

fn encode_state(state: &LifecycleState) -> Result<EncodedState> {
    match state {
        LifecycleState::Forking => Ok(("Forking", None, None, None)),
        LifecycleState::Running => Ok(("Running", None, None, None)),
        LifecycleState::Exited(exit) => Ok(("Exited", exit.code, exit.signal, None)),
        LifecycleState::Lost(evidence) => Ok(("Lost", None, None, Some(encode_lost(*evidence)?))),
        _ => Err(anyhow!("unsupported lifecycle state variant: {state:?}")),
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

fn encode_lost(evidence: LostEvidence) -> Result<&'static str> {
    match evidence {
        LostEvidence::ShimDiedBeforeReport => Ok("ShimDiedBeforeReport"),
        LostEvidence::PidNotAlive => Ok("PidNotAlive"),
        LostEvidence::PidReuseDetected => Ok("PidReuseDetected"),
        _ => Err(anyhow!("unsupported lost evidence variant: {evidence:?}")),
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

fn push_where(query: &mut QueryBuilder<'_, Sqlite>, has_where: &mut bool) {
    if *has_where {
        query.push(" AND ");
    } else {
        query.push(" WHERE ");
        *has_where = true;
    }
}

#[cfg(test)]
mod tests;
