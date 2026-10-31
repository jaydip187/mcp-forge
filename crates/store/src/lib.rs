//! SQLite-backed store for projects, tools, auth configs, DB connections and
//! UI accounts.

mod api_keys;
mod audit;
mod auth;
mod bundle;
mod calls;
mod cipher;
mod connections;
mod mcp_oauth;
mod oauth;
mod projects;
mod specs;
mod tools;
mod users;

pub use api_keys::{key_hash, ApiKey, ApiKeyInput, KEY_PREFIX};
pub use audit::MAX_AUDIT_PAGE;
pub use bundle::{BundleReport, BundleTool, ConflictPolicy};
pub use calls::{CallFilter, CallStat, MAX_CALL_PAGE};
pub use mcp_oauth::{IssuedTokens, OAuthClient, OAuthGrant, ACCESS_PREFIX, REFRESH_PREFIX};
pub use oauth::{OAuthStatus, OAuthToken};
pub use specs::{NewSpec, ReimportResult, SpecRecord};
pub use users::{Backoff, Session, User};

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
    /// Encrypts stored OAuth tokens.
    cipher: std::sync::Arc<cipher::Cipher>,
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
        let cipher = cipher::Cipher::load_or_create(data_dir)?;
        Self::connect(options, cipher).await
    }

    /// Opens a private in-memory database, mainly for tests.
    pub async fn open_in_memory() -> Result<Self, StoreError> {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")?.foreign_keys(true);
        Self::connect(options, cipher::Cipher::ephemeral()).await
    }

    async fn connect(
        options: SqliteConnectOptions,
        cipher: cipher::Cipher,
    ) -> Result<Self, StoreError> {
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
        Ok(Self {
            pool,
            cipher: std::sync::Arc::new(cipher),
        })
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
#[path = "../tests/unit/lib.rs"]
mod tests;
