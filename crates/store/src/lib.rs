//! SQLite-backed store for projects, tools and auth configs.

mod auth;
mod projects;
mod specs;
mod tools;

pub use specs::{NewSpec, SpecRecord};

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use chrono::{DateTime, SubsecRound, Utc};
use forge_core::ValidationErrors;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};
use uuid::Uuid;

/// File name of the internal database inside the data directory.
pub const DB_FILE: &str = "mcp-forge.db";

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("{0} not found")]
    NotFound(&'static str),
    #[error("{0}")]
    Conflict(String),
    #[error("invalid input: {0}")]
    Validation(#[from] ValidationErrors),
    #[error("stored data is corrupt: {0}")]
    Corrupt(String),
    #[error("failed to create data directory {path}: {source}")]
    DataDir {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
}

/// Maps a unique-constraint violation to `Conflict(msg)`; other errors pass through.
fn on_unique(e: sqlx::Error, msg: impl FnOnce() -> String) -> StoreError {
    match &e {
        sqlx::Error::Database(db) if db.is_unique_violation() => StoreError::Conflict(msg()),
        _ => StoreError::Db(e),
    }
}

/// Current time at millisecond precision, so values round-trip through SQLite unchanged.
fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(3)
}

fn parse_id(s: &str) -> Result<Uuid, StoreError> {
    Uuid::parse_str(s).map_err(|e| StoreError::Corrupt(format!("bad id {s:?}: {e}")))
}

fn from_json<T: serde::de::DeserializeOwned>(s: &str, what: &str) -> Result<T, StoreError> {
    serde_json::from_str(s).map_err(|e| StoreError::Corrupt(format!("bad {what} JSON: {e}")))
}

fn to_json<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_string(v).expect("IR types always serialize")
}

#[derive(Clone, Debug)]
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    /// Opens (creating if needed) the database in `data_dir` and runs pending migrations.
    pub async fn open(data_dir: &Path) -> Result<Self, StoreError> {
        std::fs::create_dir_all(data_dir).map_err(|source| StoreError::DataDir {
            path: data_dir.to_path_buf(),
            source,
        })?;
        let options = SqliteConnectOptions::new()
            .filename(data_dir.join(DB_FILE))
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5));
        Self::connect(options).await
    }

    /// Opens a private in-memory database, mainly for tests.
    pub async fn open_in_memory() -> Result<Self, StoreError> {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")?.foreign_keys(true);
        Self::connect(options).await
    }

    async fn connect(options: SqliteConnectOptions) -> Result<Self, StoreError> {
        // A single connection keeps in-memory databases coherent; file-backed
        // stores get a small pool.
        let max = if options.get_filename() == Path::new(":memory:") {
            1
        } else {
            5
        };
        let pool = SqlitePoolOptions::new()
            .max_connections(max)
            .connect_with(options)
            .await?;
        MIGRATOR.run(&pool).await?;
        tracing::debug!("store ready, migrations applied");
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Cheap liveness check used by `/healthz`.
    pub async fn ping(&self) -> Result<(), StoreError> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn open_creates_db_file_and_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("nested").join("data");
        let store = Store::open(&dir).await.unwrap();
        assert!(dir.join(DB_FILE).exists());
        store.ping().await.unwrap();
    }

    #[tokio::test]
    async fn reopen_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        Store::open(tmp.path()).await.unwrap().pool().close().await;
        Store::open(tmp.path()).await.unwrap().ping().await.unwrap();
    }

    #[tokio::test]
    async fn in_memory_works() {
        Store::open_in_memory().await.unwrap().ping().await.unwrap();
    }
}
