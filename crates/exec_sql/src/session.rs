//! Layer 2 of the read-only model: every query runs inside a read-only
//! transaction with a server-side timeout, then rolls back.
//!
//! - Postgres: connections start with `default_transaction_read_only=on`;
//!   each run is `BEGIN READ ONLY` + `SET LOCAL statement_timeout`.
//! - MySQL: `SET SESSION max_execution_time` (MariaDB: `max_statement_time`)
//!   + `START TRANSACTION READ ONLY`.
//! - SQLite: the file is opened `mode=ro` with `PRAGMA query_only = ON`; a
//!   progress handler interrupts statements that run past the deadline.
//!
//! A client-side timer backs up every server-side timeout.

use std::time::{Duration, Instant};

use futures_util::TryStreamExt;
use serde_json::Value;
use sqlx::mysql::MySqlArguments;
use sqlx::mysql::MySqlPool;
use sqlx::postgres::PgArguments;
use sqlx::postgres::PgPool;
use sqlx::sqlite::SqliteArguments;
use sqlx::sqlite::SqlitePool;
use sqlx::{Arguments, Column, Connection, Executor, Row, Statement};

use crate::{value, SqlError};

/// A connection pool for one configured database.
#[derive(Debug, Clone)]
pub enum DbPool {
    Sqlite(SqlitePool),
    Postgres(PgPool),
    Mysql(MySqlPool),
}

impl DbPool {
    pub async fn close(&self) {
        match self {
            DbPool::Sqlite(p) => p.close().await,
            DbPool::Postgres(p) => p.close().await,
            DbPool::Mysql(p) => p.close().await,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub row_limit: u32,
    pub timeout_ms: u32,
}

/// A value bound to a positional placeholder.
#[derive(Debug, Clone, PartialEq)]
pub enum Bind {
    /// SQL NULL, typed after the param's declared JSON type.
    Null(&'static str),
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
}

impl Bind {
    pub fn to_json(&self) -> Value {
        match self {
            Bind::Null(_) => Value::Null,
            Bind::Bool(b) => Value::Bool(*b),
            Bind::Int(i) => Value::from(*i),
            Bind::Float(f) => serde_json::Number::from_f64(*f).map_or(Value::Null, Value::Number),
            Bind::Text(s) => Value::String(s.clone()),
        }
    }
}

/// Builds the driver's argument list for `query_with`.
macro_rules! args_for {
    ($args:ty, $binds:expr) => {{
        let mut a = <$args>::default();
        for b in $binds {
            match b {
                Bind::Null("integer") => a.add(None::<i64>),
                Bind::Null("number") => a.add(None::<f64>),
                Bind::Null("boolean") => a.add(None::<bool>),
                Bind::Null(_) => a.add(None::<String>),
                Bind::Bool(v) => a.add(*v),
                Bind::Int(v) => a.add(*v),
                Bind::Float(v) => a.add(*v),
                Bind::Text(v) => a.add(v.clone()),
            }
            .map_err(|e| SqlError::Database(format!("could not bind a value: {e}")))?;
        }
        a
    }};
}

/// Rows read by one run, before masking.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    /// More rows existed than `row_limit`.
    pub truncated: bool,
}

/// Runs `sql` read-only. This is the session layer alone: it does **not**
/// check the statement (see [`crate::guard`]), so the database itself must
/// refuse writes. Public so tests can prove that.
pub async fn run_read_only(
    pool: &DbPool,
    sql: &str,
    binds: &[Bind],
    limits: Limits,
) -> Result<RawResult, SqlError> {
    let budget = Duration::from_millis(u64::from(limits.timeout_ms));
    // The server-side timeout should fire first; this catches a stuck network.
    let backstop = budget + Duration::from_secs(2);
    let run = async {
        match pool {
            DbPool::Postgres(p) => run_pg(p, sql, binds, limits).await,
            DbPool::Mysql(p) => run_mysql(p, sql, binds, limits).await,
            DbPool::Sqlite(p) => run_sqlite(p, sql, binds, limits, budget).await,
        }
    };
    match tokio::time::timeout(backstop, run).await {
        Ok(r) => r,
        Err(_) => Err(SqlError::Timeout(limits.timeout_ms)),
    }
}

/// Collects up to `limit` rows; `truncated` if there was at least one more.
macro_rules! collect_rows {
    ($stream:expr, $limit:expr, $convert:path) => {{
        let mut stream = $stream;
        let mut columns: Option<Vec<String>> = None;
        let mut rows = Vec::new();
        let mut truncated = false;
        while let Some(row) = stream.try_next().await? {
            if columns.is_none() {
                columns = Some(row.columns().iter().map(|c| c.name().to_owned()).collect());
            }
            if rows.len() as u32 >= $limit {
                truncated = true;
                break;
            }
            rows.push($convert(&row));
        }
        (columns, rows, truncated)
    }};
}

async fn run_pg(
    pool: &PgPool,
    sql: &str,
    binds: &[Bind],
    limits: Limits,
) -> Result<RawResult, SqlError> {
    let mut tx = pool
        .begin_with("BEGIN READ ONLY")
        .await
        .map_err(SqlError::connect)?;
    let set = format!("SET LOCAL statement_timeout = {}", limits.timeout_ms);
    (&mut *tx).execute(set.as_str()).await?;
    let args = args_for!(PgArguments, binds);
    let (columns, rows, truncated) = collect_rows!(
        sqlx::query_with(sql, args).fetch(&mut *tx),
        limits.row_limit,
        value::pg_row
    );
    let columns = match columns {
        Some(c) => c,
        None => names(tx.prepare(sql).await?.columns()),
    };
    tx.rollback().await?;
    Ok(RawResult {
        columns,
        rows,
        truncated,
    })
}

async fn run_mysql(
    pool: &MySqlPool,
    sql: &str,
    binds: &[Bind],
    limits: Limits,
) -> Result<RawResult, SqlError> {
    let mut conn = pool.acquire().await.map_err(SqlError::connect)?;
    let set = format!("SET SESSION max_execution_time = {}", limits.timeout_ms);
    if let Err(e) = (&mut *conn).execute(set.as_str()).await {
        // MariaDB has no max_execution_time (1193 = unknown variable).
        let unknown = matches!(&e, sqlx::Error::Database(d)
            if d.try_downcast_ref::<sqlx::mysql::MySqlDatabaseError>().is_some_and(|m| m.number() == 1193));
        if !unknown {
            return Err(e.into());
        }
        let secs = f64::from(limits.timeout_ms) / 1000.0;
        let set = format!("SET SESSION max_statement_time = {secs}");
        (&mut *conn).execute(set.as_str()).await?;
    }
    let mut tx = conn.begin_with("START TRANSACTION READ ONLY").await?;
    let args = args_for!(MySqlArguments, binds);
    let (columns, rows, truncated) = collect_rows!(
        sqlx::query_with(sql, args).fetch(&mut *tx),
        limits.row_limit,
        value::mysql_row
    );
    let columns = match columns {
        Some(c) => c,
        None => names(tx.prepare(sql).await?.columns()),
    };
    tx.rollback().await?;
    Ok(RawResult {
        columns,
        rows,
        truncated,
    })
}

async fn run_sqlite(
    pool: &SqlitePool,
    sql: &str,
    binds: &[Bind],
    limits: Limits,
    budget: Duration,
) -> Result<RawResult, SqlError> {
    let mut conn = pool.acquire().await.map_err(SqlError::connect)?;
    let deadline = Instant::now() + budget;
    // Returning false interrupts the running statement (SQLITE_INTERRUPT).
    conn.lock_handle()
        .await?
        .set_progress_handler(1_000, move || Instant::now() < deadline);
    let result = async {
        let mut tx = conn.begin().await?;
        let args = args_for!(SqliteArguments, binds);
        let (columns, rows, truncated) = collect_rows!(
            sqlx::query_with(sql, args).fetch(&mut *tx),
            limits.row_limit,
            value::sqlite_row
        );
        let columns = match columns {
            Some(c) => c,
            None => names(tx.prepare(sql).await?.columns()),
        };
        tx.rollback().await?;
        Ok::<_, SqlError>(RawResult {
            columns,
            rows,
            truncated,
        })
    }
    .await;
    // The handler outlives this run on the pooled connection; drop it.
    if let Ok(mut h) = conn.lock_handle().await {
        h.remove_progress_handler();
    }
    result.map_err(|e| match e {
        SqlError::Database(m) if m.contains("interrupted") => SqlError::Timeout(limits.timeout_ms),
        other => other,
    })
}

fn names<C: Column>(cols: &[C]) -> Vec<String> {
    cols.iter().map(|c| c.name().to_owned()).collect()
}
