//! SQLite store: connection setup and migrations. All SQL lives in this crate.

use std::path::Path;
use std::time::Duration;

use sqlx::SqlitePool;
use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

pub mod repository;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
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
