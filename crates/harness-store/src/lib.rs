//! SQLite store: connection setup and migrations. All SQL lives in this crate.

use std::path::Path;
use std::time::Duration;

use sqlx::SqlitePool;
use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

pub mod approvals;
pub mod budgets;
pub mod dashboard;
pub mod execution;
pub mod git_artifacts;
pub mod leases;
pub mod plans;
pub mod repository;
pub mod trust;
pub mod usage;
pub mod verification;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("budget denied: {0}")]
    BudgetDenied(&'static str),
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("{0} not found")]
    NotFound(&'static str),
    #[error("already exists")]
    AlreadyExists,
    #[error("revision conflict: expected {expected}, actual {actual}")]
    RevisionConflict { expected: u64, actual: u64 },
    #[error("idempotency key already used by a different request")]
    IdempotencyKeyReused,
    #[error("task in state {0:?} cannot be reprioritized")]
    NotReprioritizable(harness_core::domain::TaskState),
    #[error("{0}")]
    IllegalTransition(harness_core::state::TransitionError),
    #[error("invalid domain value: {0}")]
    Domain(#[from] harness_core::domain::DomainError),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("corrupt stored data: {0}")]
    Corrupt(String),
    #[error("{0}")]
    Plan(#[from] harness_core::plan::PlanError),
    #[error("plan cannot be decided: {0}")]
    PlanNotDecidable(&'static str),
}

impl StoreError {
    /// Map unique-constraint violations to `AlreadyExists`, keep everything else as-is.
    pub(crate) fn from_insert(e: sqlx::Error) -> Self {
        match &e {
            sqlx::Error::Database(d) if d.is_unique_violation() => Self::AlreadyExists,
            _ => Self::Db(e),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    /// Persist a bounded, redacted worker failure for operator diagnostics.
    pub async fn record_task_failure(
        &self,
        run: &harness_core::domain::RunId,
        task: &harness_core::domain::TaskId,
        attempt: u32,
        reason: &str,
    ) -> Result<(), StoreError> {
        sqlx::query("INSERT INTO events (run_id, task_id, kind, attempt, payload_json) VALUES (?, ?, 'task.failure', ?, ?)")
            .bind(run.as_str())
            .bind(task.as_str())
            .bind(i64::from(attempt))
            .bind(serde_json::json!({"reason":reason}).to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Append a run-level diagnostic event (e.g. main-agent planning outcome).
    pub async fn record_run_event(
        &self,
        run: &harness_core::domain::RunId,
        kind: &str,
        payload: &serde_json::Value,
    ) -> Result<(), StoreError> {
        sqlx::query("INSERT INTO events (run_id, kind, payload_json) VALUES (?, ?, ?)")
            .bind(run.as_str())
            .bind(kind)
            .bind(payload.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Most recent run-level event whose kind starts with `prefix`, as (kind, payload).
    pub async fn latest_run_event(
        &self,
        run: &harness_core::domain::RunId,
        prefix: &str,
    ) -> Result<Option<(String, serde_json::Value)>, StoreError> {
        let row: Option<(String, String)> = sqlx::query_as("SELECT kind, payload_json FROM events WHERE run_id = ? AND task_id IS NULL AND substr(kind, 1, length(?)) = ? ORDER BY sequence DESC LIMIT 1")
            .bind(run.as_str()).bind(prefix).bind(prefix).fetch_optional(&self.pool).await?;
        Ok(row.map(|(kind, payload)| {
            (
                kind,
                serde_json::from_str(&payload).unwrap_or(serde_json::Value::Null),
            )
        }))
    }

    pub async fn latest_task_failure(
        &self,
        run: &harness_core::domain::RunId,
        task: &harness_core::domain::TaskId,
    ) -> Result<Option<String>, StoreError> {
        let payload: Option<String> = sqlx::query_scalar("SELECT payload_json FROM events WHERE run_id = ? AND task_id = ? AND kind = 'task.failure' ORDER BY sequence DESC LIMIT 1")
            .bind(run.as_str()).bind(task.as_str()).fetch_optional(&self.pool).await?;
        Ok(payload
            .and_then(|p| serde_json::from_str::<serde_json::Value>(&p).ok())
            .and_then(|v| v["reason"].as_str().map(str::to_owned)))
    }
    /// Open (creating if needed) a database file and apply all migrations.
    ///
    /// Always WAL, foreign keys enforced, 5s busy timeout. Do not place the file
    /// on a network mount: WAL requires shared memory on a local filesystem.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await?;
        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    /// Idempotent: already-applied migrations are skipped.
    pub async fn migrate(&self) -> Result<(), StoreError> {
        MIGRATOR.run(&self.pool).await?;
        Ok(())
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }
}
