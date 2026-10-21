//! SQLite-backed store for projects, tools and auth configs.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};

/// File name of the internal database inside the data directory.
pub const DB_FILE: &str = "mcp-forge.db";

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
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
