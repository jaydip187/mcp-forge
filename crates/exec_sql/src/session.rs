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

/// How a write runs.
#[derive(Debug, Clone, Copy)]
pub struct WriteOptions {
    /// More changed rows than this rolls the statement back.
    pub max_affected: u32,
    /// Always roll back; only report the count.
    pub dry_run: bool,
    /// The statement has `RETURNING`: its rows are the changed rows.
    pub returning: bool,
}

/// What a write did. `rows` holds `RETURNING` output (up to the row limit).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WriteResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub truncated: bool,
    pub affected: u64,
    pub committed: bool,
}

/// Runs inside an open transaction: executes, counts changed rows, then
/// rolls back (over the cap, or a dry run) or commits.
macro_rules! write_in_tx {
    ($tx:expr, $sql:expr, $args:expr, $limits:expr, $opts:expr, $convert:path) => {{
        let mut columns: Vec<String> = Vec::new();
        let mut rows = Vec::new();
        let mut truncated = false;
        let max = u64::from($opts.max_affected);
        let mut at_least = false;
        let affected: u64 = if $opts.returning {
            let mut stream = sqlx::query_with($sql, $args).fetch(&mut *$tx);
            let mut n: u64 = 0;
            while let Some(row) = stream.try_next().await? {
                n += 1;
                if columns.is_empty() {
                    columns = row.columns().iter().map(|c| c.name().to_owned()).collect();
                }
                if n > max {
                    // Over the cap: no need to read the rest.
                    at_least = true;
                    break;
                }
                if rows.len() < $limits.row_limit as usize {
                    rows.push($convert(&row));
                } else {
                    truncated = true;
                }
            }
            n
        } else {
            sqlx::query_with($sql, $args)
                .execute(&mut *$tx)
                .await?
                .rows_affected()
        };
        if affected > max {
            $tx.rollback().await?;
            return Err(SqlError::TooManyRows {
                affected,
                max: $opts.max_affected,
                at_least,
            });
        }
        let committed = if $opts.dry_run {
            $tx.rollback().await?;
            false
        } else {
            $tx.commit().await?;
            true
        };
        WriteResult {
            columns,
            rows,
            truncated,
            affected,
            committed,
        }
    }};
}

/// Runs one data-changing statement in a read-write transaction with the
/// connection's time limit. Nothing is committed when it changes more than
/// `max_affected` rows or on a dry run. The caller must have checked and
/// gated the statement; on SQLite the pool must be the writable one.
pub async fn run_write(
    pool: &DbPool,
    sql: &str,
    binds: &[Bind],
    limits: Limits,
    opts: WriteOptions,
) -> Result<WriteResult, SqlError> {
    let budget = Duration::from_millis(u64::from(limits.timeout_ms));
    let backstop = budget + Duration::from_secs(2);
    let run = async {
        match pool {
            DbPool::Postgres(p) => write_pg(p, sql, binds, limits, opts).await,
            DbPool::Mysql(p) => write_mysql(p, sql, binds, limits, opts).await,
            DbPool::Sqlite(p) => write_sqlite(p, sql, binds, limits, opts, budget).await,
        }
    };
    // A dropped transaction rolls back, so a timeout never commits.
    match tokio::time::timeout(backstop, run).await {
        Ok(r) => r,
        Err(_) => Err(SqlError::Timeout(limits.timeout_ms)),
    }
}

async fn write_pg(
    pool: &PgPool,
    sql: &str,
    binds: &[Bind],
    limits: Limits,
    opts: WriteOptions,
) -> Result<WriteResult, SqlError> {
    // Pools default to read-only transactions; writes opt in explicitly.
    let mut tx = pool
        .begin_with("BEGIN READ WRITE")
        .await
        .map_err(SqlError::connect)?;
    let set = format!("SET LOCAL statement_timeout = {}", limits.timeout_ms);
    (&mut *tx).execute(set.as_str()).await?;
    let args = args_for!(PgArguments, binds);
    Ok(write_in_tx!(tx, sql, args, limits, opts, value::pg_row))
}

async fn write_mysql(
    pool: &MySqlPool,
    sql: &str,
    binds: &[Bind],
    limits: Limits,
    opts: WriteOptions,
) -> Result<WriteResult, SqlError> {
    let mut conn = pool.acquire().await.map_err(SqlError::connect)?;
    // max_execution_time only covers SELECT; bound lock waits instead and
    // rely on the client-side timer for the rest.
    let secs = limits.timeout_ms.div_ceil(1000).max(1);
    let set = format!("SET SESSION innodb_lock_wait_timeout = {secs}");
    (&mut *conn).execute(set.as_str()).await?;
    let mut tx = conn.begin_with("START TRANSACTION READ WRITE").await?;
    let args = args_for!(MySqlArguments, binds);
    Ok(write_in_tx!(tx, sql, args, limits, opts, value::mysql_row))
}

async fn write_sqlite(
    pool: &SqlitePool,
    sql: &str,
    binds: &[Bind],
    limits: Limits,
    opts: WriteOptions,
    budget: Duration,
) -> Result<WriteResult, SqlError> {
    let mut conn = pool.acquire().await.map_err(SqlError::connect)?;
    let deadline = Instant::now() + budget;
    conn.lock_handle()
        .await?
        .set_progress_handler(1_000, move || Instant::now() < deadline);
    let result = async {
        let mut tx = conn.begin().await?;
        let args = args_for!(SqliteArguments, binds);
        Ok::<_, SqlError>(write_in_tx!(tx, sql, args, limits, opts, value::sqlite_row))
    }
    .await;
    if let Ok(mut h) = conn.lock_handle().await {
        h.remove_progress_handler();
    }
    result.map_err(|e| match e {
        SqlError::Database(m) if m.contains("interrupted") => SqlError::Timeout(limits.timeout_ms),
        other => other,
    })
}
