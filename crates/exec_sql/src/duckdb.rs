//! DuckDB, embedded. Read-only connections open the file read-only; every
//! read also runs in a transaction that is rolled back. On open, queries are
//! limited to the connection's allowed folders (`allowed_directories` with
//! external access off), extensions can't be installed or auto-loaded (so no
//! network access), and the configuration is locked so a query can't undo
//! any of it. A timer interrupts queries that run past the time limit.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use duckdb::types::{TimeUnit, ToSqlOutput, Value as Dv};
use duckdb::{AccessMode, Config, Connection, ToSql};
use serde_json::{json, Map, Value};

use crate::value::cap_cell;
use crate::{Bind, Limits, RawResult, SqlError, WriteOptions, WriteResult};

/// One DuckDB database. DuckDB runs queries on its own threads; calls into
/// it are made from blocking tasks, one at a time per connection.
#[derive(Clone)]
pub struct DuckdbPool {
    conn: Arc<Mutex<Connection>>,
    /// Opened read-write (the connection allows writes).
    writable: bool,
    /// In-memory database: nothing persists anyway.
    memory: bool,
}

impl std::fmt::Debug for DuckdbPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DuckdbPool")
            .field("writable", &self.writable)
            .field("memory", &self.memory)
            .finish_non_exhaustive()
    }
}

fn db_err(e: duckdb::Error) -> SqlError {
    let m = e.to_string();
    if m.contains("INTERRUPT") || m.contains("Interrupted") {
        SqlError::Timeout(0)
    } else {
        SqlError::Database(m)
    }
}

/// `'a', 'b'` as a DuckDB list literal (folders are validated: no quotes).
fn list(items: &[String]) -> String {
    let quoted: Vec<String> = items
        .iter()
        .map(|i| format!("'{}'", i.replace('\\', "/").replace('\'', "''")))
        .collect();
    format!("[{}]", quoted.join(", "))
}

impl DuckdbPool {
    /// `dsn` is a file path (optionally `duckdb:`-prefixed) or `:memory:`.
    pub(crate) fn open(dsn: &str, writable: bool, folders: &[String]) -> Result<Self, SqlError> {
        let path = dsn
            .trim()
            .trim_start_matches("duckdb://")
            .trim_start_matches("duckdb:");
        let memory = path.is_empty() || path == ":memory:";
        let config = |read_only: bool| -> Result<Config, SqlError> {
            let c = Config::default()
                .enable_autoload_extension(false)
                .map_err(db_err)?;
            if read_only {
                c.access_mode(AccessMode::ReadOnly).map_err(db_err)
            } else {
                Ok(c)
            }
        };
        let conn = if memory {
            Connection::open_in_memory_with_flags(config(false)?)
        } else {
            if !std::path::Path::new(path).exists() {
                return Err(SqlError::Connect(format!("no DuckDB file at {path}")));
            }
            Connection::open_with_flags(path, config(!writable)?)
        }
        .map_err(|e| SqlError::Connect(e.to_string()))?;
        // Order matters: allowed_directories can't change once external
        // access is off, and nothing can once the configuration is locked.
        let setup = format!(
            "SET autoinstall_known_extensions = false; \
             SET autoload_known_extensions = false; \
             SET allowed_directories = {}; \
             SET enable_external_access = false; \
             SET lock_configuration = true;",
            list(folders)
        );
        conn.execute_batch(&setup)
            .map_err(|e| SqlError::Connect(format!("could not restrict DuckDB: {e}")))?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            writable: writable && !memory,
            memory,
        })
    }

    /// Runs `f` on a blocking thread, interrupting DuckDB after `timeout_ms`.
    async fn with_conn<T: Send + 'static>(
        &self,
        timeout_ms: u32,
        f: impl FnOnce(&Connection) -> Result<T, SqlError> + Send + 'static,
    ) -> Result<T, SqlError> {
        let conn = self.conn.clone();
        let handle = {
            let c = conn.lock().expect("duckdb connection poisoned");
            c.interrupt_handle()
        };
        let timer = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(u64::from(timeout_ms))).await;
            handle.interrupt();
        });
        let result = tokio::task::spawn_blocking(move || {
            let c = conn.lock().expect("duckdb connection poisoned");
            f(&c)
        })
        .await
        .map_err(|e| SqlError::Database(format!("DuckDB task failed: {e}")))?;
        timer.abort();
        result
    }
}

/// A bind as a DuckDB parameter.
struct P<'a>(&'a Bind);

impl ToSql for P<'_> {
    fn to_sql(&self) -> duckdb::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::Owned(match self.0 {
            Bind::Null(_) => Dv::Null,
            Bind::Bool(v) => Dv::Boolean(*v),
            Bind::Int(v) => Dv::BigInt(*v),
            Bind::Float(v) => Dv::Double(*v),
            Bind::Text(v) => Dv::Text(v.clone()),
        }))
    }
}

fn micros(unit: TimeUnit, v: i64) -> i64 {
    match unit {
        TimeUnit::Second => v.saturating_mul(1_000_000),
        TimeUnit::Millisecond => v.saturating_mul(1_000),
        TimeUnit::Microsecond => v,
        TimeUnit::Nanosecond => v / 1_000,
    }
}

fn float(f: f64) -> Value {
    serde_json::Number::from_f64(f).map_or_else(|| Value::String(f.to_string()), Value::Number)
}

/// One DuckDB value as JSON.
pub(crate) fn to_json(v: Dv) -> Value {
    match v {
        Dv::Null => Value::Null,
        Dv::Boolean(b) => Value::Bool(b),
        Dv::TinyInt(i) => json!(i),
        Dv::SmallInt(i) => json!(i),
        Dv::Int(i) => json!(i),
        Dv::BigInt(i) => json!(i),
        Dv::UTinyInt(i) => json!(i),
        Dv::USmallInt(i) => json!(i),
        Dv::UInt(i) => json!(i),
        Dv::UBigInt(i) => json!(i),
        // Beyond i64 / f64 precision: keep every digit as text.
        Dv::HugeInt(i) => Value::String(i.to_string()),
        Dv::Float(f) => float(f64::from(f)),
        Dv::Double(f) => float(f),
        Dv::Decimal(d) => Value::String(d.to_string()),
        Dv::Text(s) | Dv::Enum(s) => Value::String(s),
        Dv::Blob(b) => Value::String(format!("<binary, {} bytes>", b.len())),
        Dv::Timestamp(unit, t) => chrono::DateTime::from_timestamp_micros(micros(unit, t))
            .map_or(Value::Null, |d| Value::String(d.to_rfc3339())),
        Dv::Date32(days) => chrono::NaiveDate::from_num_days_from_ce_opt(days + 719_163)
            .map_or(Value::Null, |d| Value::String(d.to_string())),
        Dv::Time64(unit, t) => {
            let us = micros(unit, t);
            chrono::NaiveTime::from_num_seconds_from_midnight_opt(
                (us / 1_000_000) as u32,
                ((us % 1_000_000) * 1_000) as u32,
            )
            .map_or(Value::Null, |t| Value::String(t.to_string()))
        }
        Dv::List(items) | Dv::Array(items) => {
            Value::Array(items.into_iter().map(to_json).collect())
        }
        Dv::Struct(fields) => Value::Object(
            fields
                .iter()
                .map(|(k, v)| (k.clone(), to_json(v.clone())))
                .collect::<Map<String, Value>>(),
        ),
        other => Value::String(format!("{other:?}")),
    }
}

/// Runs a query and reads up to `limit + 1` rows.
fn read(
    c: &Connection,
    sql: &str,
    binds: &[Bind],
    limit: u32,
) -> Result<(Vec<String>, Vec<Vec<Value>>), SqlError> {
    let mut stmt = c.prepare(sql).map_err(db_err)?;
    let params: Vec<P> = binds.iter().map(P).collect();
    let mut rows = stmt
        .query(duckdb::params_from_iter(params.iter()))
        .map_err(db_err)?;
    let columns: Vec<String> = rows.as_ref().map(|s| s.column_names()).unwrap_or_default();
    let n = columns.len();
    let mut out = Vec::new();
    while let Some(row) = rows.next().map_err(db_err)? {
        let mut cells = Vec::with_capacity(n);
        for i in 0..n {
            let v: Dv = row.get(i).map_err(db_err)?;
            cells.push(cap_cell(to_json(v)));
        }
        out.push(cells);
        if out.len() > limit as usize {
            break;
        }
    }
    Ok((columns, out))
}

pub(crate) async fn run_read_only(
    pool: &DuckdbPool,
    sql: &str,
    binds: &[Bind],
    limits: Limits,
) -> Result<RawResult, SqlError> {
    let (sql, binds) = (sql.to_owned(), binds.to_vec());
    let limit = limits.row_limit;
    pool.with_conn(limits.timeout_ms, move |c| {
        c.execute_batch("BEGIN TRANSACTION").map_err(db_err)?;
        let result = read(c, &sql, &binds, limit);
        // Nothing a read did is kept.
        let rolled = c.execute_batch("ROLLBACK").map_err(db_err);
        let (columns, mut rows) = result?;
        rolled?;
        let truncated = rows.len() > limit as usize;
        rows.truncate(limit as usize);
        Ok(RawResult {
            columns,
            rows,
            truncated,
        })
    })
    .await
}

pub(crate) async fn run_write(
    pool: &DuckdbPool,
    sql: &str,
    binds: &[Bind],
    limits: Limits,
    opts: WriteOptions,
) -> Result<WriteResult, SqlError> {
    if !pool.writable {
        return Err(SqlError::Rejected(
            "this DuckDB connection is read-only".into(),
        ));
    }
    let (sql, binds) = (sql.to_owned(), binds.to_vec());
    pool.with_conn(limits.timeout_ms, move |c| {
        c.execute_batch("BEGIN TRANSACTION").map_err(db_err)?;
        let ran = (|| -> Result<(u64, Vec<String>, Vec<Vec<Value>>), SqlError> {
            if opts.returning {
                let (cols, rows) = read(c, &sql, &binds, u32::MAX - 1)?;
                Ok((rows.len() as u64, cols, rows))
            } else {
                let mut stmt = c.prepare(&sql).map_err(db_err)?;
                let params: Vec<P> = binds.iter().map(P).collect();
                let n = stmt
                    .execute(duckdb::params_from_iter(params.iter()))
                    .map_err(db_err)?;
                Ok((n as u64, Vec::new(), Vec::new()))
            }
        })();
        let (affected, columns, mut rows) = match ran {
            Ok(r) => r,
            Err(e) => {
                let _ = c.execute_batch("ROLLBACK");
                return Err(e);
            }
        };
        let max = u64::from(opts.max_affected);
        let keep = affected <= max && !opts.dry_run;
        c.execute_batch(if keep { "COMMIT" } else { "ROLLBACK" })
            .map_err(db_err)?;
        if affected > max {
            return Err(SqlError::TooManyRows {
                affected,
                max: opts.max_affected,
                at_least: false,
            });
        }
        let truncated = rows.len() > limits.row_limit as usize;
        rows.truncate(limits.row_limit as usize);
        Ok(WriteResult {
            columns,
            rows,
            truncated,
            affected,
            committed: keep,
        })
    })
    .await
}

pub(crate) async fn probe(pool: &DuckdbPool) -> Result<crate::pool::ConnectionReport, SqlError> {
    let raw = run_read_only(
        pool,
        "SELECT version()",
        &[],
        Limits {
            row_limit: 1,
            timeout_ms: 10_000,
        },
    )
    .await?;
    let version = raw
        .rows
        .first()
        .and_then(|r| r.first())
        .and_then(Value::as_str)
        .map(|v| format!("DuckDB {v}"));
    let mut r = crate::pool::ConnectionReport {
        server_version: version,
        ..Default::default()
    };
    if pool.memory {
        r.can_write = Some(false);
        r.warnings
            .push("In-memory database: queries can read allowed files; nothing is stored.".into());
    } else if pool.writable {
        r.can_write = Some(true);
        r.write_privileges
            .push("the database file is opened read-write (saved writes are on)".into());
    } else {
        r.can_write = Some(false);
    }
    Ok(r)
}

#[cfg(test)]
#[path = "../tests/unit/duckdb.rs"]
mod tests;
