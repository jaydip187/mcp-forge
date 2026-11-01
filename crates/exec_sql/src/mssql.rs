//! SQL Server (and Azure SQL) through tiberius. SQL Server has no read-only
//! transaction, so reads rely on the statement guard and run inside a
//! transaction that is always rolled back, with `SET ROWCOUNT` capping rows
//! and `SET LOCK_TIMEOUT` bounding lock waits. "Test connection" reports
//! whether the login can write; a `db_datareader`-only login is recommended.

use std::sync::{Arc, Mutex};

use futures_util::TryStreamExt;
use serde_json::{json, Value};
use tiberius::{AuthMethod, Client, ColumnData, Config, EncryptionLevel, FromSql, Query};
use tokio::net::TcpStream;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

use crate::value::cap_cell;
use crate::{Bind, Limits, RawResult, SqlError, WriteOptions, WriteResult};

type Conn = Client<Compat<TcpStream>>;

/// Idle connections kept for reuse.
const IDLE: usize = 4;

/// A few idle connections. One is put back only after a run finished
/// cleanly, so a connection abandoned mid-query (timeout) is never reused.
#[derive(Clone)]
pub struct MssqlPool {
    config: Config,
    idle: Arc<Mutex<Vec<Conn>>>,
}

impl std::fmt::Debug for MssqlPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MssqlPool").finish_non_exhaustive()
    }
}

/// `mssql://user:password@host:1433/database?trust_server_certificate=true`
/// (also `sqlserver://`), an ADO.NET string (`Server=…;User Id=…`) or a JDBC
/// string (`jdbc:sqlserver://…`).
pub(crate) fn parse_dsn(dsn: &str) -> Result<Config, SqlError> {
    let dsn = dsn.trim();
    let bad = |e: String| SqlError::Config(format!("invalid connection string: {e}"));
    let lower = dsn.to_ascii_lowercase();
    if lower.starts_with("jdbc:sqlserver://") {
        return Config::from_jdbc_string(dsn).map_err(|e| bad(e.to_string()));
    }
    if !(lower.starts_with("mssql://") || lower.starts_with("sqlserver://")) {
        if dsn.contains('=') && dsn.contains(';') || lower.starts_with("server=") {
            return Config::from_ado_string(dsn).map_err(|e| bad(e.to_string()));
        }
        return Err(SqlError::Config(
            "the connection string must start with mssql://, jdbc:sqlserver:// or Server=".into(),
        ));
    }
    let url = url::Url::parse(dsn).map_err(|e| bad(e.to_string()))?;
    let decode = |s: &str| crate::error::urldecode(s).unwrap_or_else(|_| s.to_owned());
    let mut c = Config::new();
    c.host(url.host_str().ok_or_else(|| bad("no host".into()))?);
    c.port(url.port().unwrap_or(1433));
    if let Some(db) = url
        .path_segments()
        .and_then(|mut p| p.next())
        .filter(|d| !d.is_empty())
    {
        c.database(decode(db));
    }
    if !url.username().is_empty() {
        c.authentication(AuthMethod::sql_server(
            decode(url.username()),
            url.password().map(decode).unwrap_or_default(),
        ));
    }
    c.encryption(EncryptionLevel::Required);
    for (k, v) in url.query_pairs() {
        match (
            k.to_ascii_lowercase().as_str(),
            v.to_ascii_lowercase().as_str(),
        ) {
            ("trust_server_certificate" | "trustservercertificate", "true") => {
                c.trust_cert();
            }
            ("encrypt", "false") => {
                c.encryption(EncryptionLevel::NotSupported);
            }
            ("application_intent" | "applicationintent", "readonly") => {
                c.readonly(true);
            }
            _ => {}
        }
    }
    c.application_name("mcp-forge");
    Ok(c)
}

fn db_err(e: tiberius::error::Error) -> SqlError {
    match e {
        tiberius::error::Error::Server(t) => SqlError::Database(t.message().to_owned()),
        tiberius::error::Error::Io { message, .. } => SqlError::Connect(message),
        tiberius::error::Error::Tls(m) => SqlError::Connect(m),
        other => SqlError::Database(other.to_string()),
    }
}

impl MssqlPool {
    pub(crate) fn new(dsn: &str) -> Result<Self, SqlError> {
        Ok(Self {
            config: parse_dsn(dsn)?,
            idle: Arc::default(),
        })
    }

    async fn connect(&self) -> Result<Conn, SqlError> {
        let mut config = self.config.clone();
        // Azure SQL may redirect once to the node that holds the database.
        for _ in 0..2 {
            let tcp = TcpStream::connect(config.get_addr())
                .await
                .map_err(|e| SqlError::Connect(e.to_string()))?;
            tcp.set_nodelay(true).ok();
            match Client::connect(config.clone(), tcp.compat_write()).await {
                Ok(c) => return Ok(c),
                Err(tiberius::error::Error::Routing { host, port }) => {
                    config.host(host);
                    config.port(port);
                }
                Err(e) => {
                    return Err(match db_err(e) {
                        SqlError::Database(m) => SqlError::Connect(m),
                        other => other,
                    })
                }
            }
        }
        Err(SqlError::Connect("too many redirects".into()))
    }

    async fn take(&self) -> Result<Conn, SqlError> {
        let idle = self.idle.lock().expect("pool poisoned").pop();
        match idle {
            Some(c) => Ok(c),
            None => self.connect().await,
        }
    }

    fn give_back(&self, c: Conn) {
        let mut idle = self.idle.lock().expect("pool poisoned");
        if idle.len() < IDLE {
            idle.push(c);
        }
    }

    pub(crate) fn close(&self) {
        self.idle.lock().expect("pool poisoned").clear();
    }
}

fn query<'a>(sql: &'a str, binds: &'a [Bind]) -> Query<'a> {
    let mut q = Query::new(sql);
    for b in binds {
        match b {
            Bind::Null("integer") => q.bind(None::<i64>),
            Bind::Null("number") => q.bind(None::<f64>),
            Bind::Null("boolean") => q.bind(None::<bool>),
            Bind::Null(_) => q.bind(None::<&str>),
            Bind::Bool(v) => q.bind(*v),
            Bind::Int(v) => q.bind(*v),
            Bind::Float(v) => q.bind(*v),
            Bind::Text(v) => q.bind(v.as_str()),
        }
    }
    q
}

async fn simple(c: &mut Conn, sql: &str) -> Result<(), SqlError> {
    c.simple_query(sql)
        .await
        .map_err(db_err)?
        .into_results()
        .await
        .map_err(db_err)?;
    Ok(())
}

/// One cell as JSON.
pub(crate) fn cell(d: &ColumnData<'static>) -> Value {
    let n = |o: Option<Value>| o.unwrap_or(Value::Null);
    let v = match d {
        ColumnData::U8(v) => n(v.map(|x| json!(x))),
        ColumnData::I16(v) => n(v.map(|x| json!(x))),
        ColumnData::I32(v) => n(v.map(|x| json!(x))),
        ColumnData::I64(v) => n(v.map(|x| json!(x))),
        ColumnData::F32(v) => n(v.map(|x| json!(x))),
        ColumnData::F64(v) => n(v.map(|x| json!(x))),
        ColumnData::Bit(v) => n(v.map(Value::Bool)),
        ColumnData::String(v) => n(v.as_ref().map(|s| Value::String(s.to_string()))),
        ColumnData::Guid(v) => n(v.map(|u| Value::String(u.to_string()))),
        ColumnData::Binary(v) => n(v
            .as_ref()
            .map(|b| Value::String(format!("<binary, {} bytes>", b.len())))),
        ColumnData::Numeric(_) => n(rust_decimal::Decimal::from_sql(d)
            .ok()
            .flatten()
            .map(|x| Value::String(x.to_string()))),
        ColumnData::Xml(v) => n(v.as_ref().map(|x| Value::String(x.to_string()))),
        ColumnData::DateTimeOffset(_) => n(chrono::DateTime::<chrono::Utc>::from_sql(d)
            .ok()
            .flatten()
            .map(|t| Value::String(t.to_rfc3339()))),
        ColumnData::Date(_) => n(chrono::NaiveDate::from_sql(d)
            .ok()
            .flatten()
            .map(|t| Value::String(t.to_string()))),
        ColumnData::Time(_) => n(chrono::NaiveTime::from_sql(d)
            .ok()
            .flatten()
            .map(|t| Value::String(t.to_string()))),
        ColumnData::DateTime(_) | ColumnData::SmallDateTime(_) | ColumnData::DateTime2(_) => {
            n(chrono::NaiveDateTime::from_sql(d)
                .ok()
                .flatten()
                .map(|t| Value::String(t.format("%Y-%m-%dT%H:%M:%S%.f").to_string())))
        }
    };
    cap_cell(v)
}

/// Reads every row of the first result set (`SET ROWCOUNT` bounds them).
async fn read_rows(
    c: &mut Conn,
    sql: &str,
    binds: &[Bind],
) -> Result<(Vec<String>, Vec<Vec<Value>>), SqlError> {
    let mut stream = query(sql, binds).query(c).await.map_err(db_err)?;
    let columns: Vec<String> = stream
        .columns()
        .await
        .map_err(db_err)?
        .map(|cols| cols.iter().map(|c| c.name().to_owned()).collect())
        .unwrap_or_default();
    let mut rows = Vec::new();
    let mut rs = stream.into_row_stream();
    while let Some(row) = rs.try_next().await.map_err(db_err)? {
        rows.push(row.cells().map(|(_, d)| cell(d)).collect());
    }
    Ok((columns, rows))
}

pub(crate) async fn run_read_only(
    pool: &MssqlPool,
    sql: &str,
    binds: &[Bind],
    limits: Limits,
) -> Result<RawResult, SqlError> {
    let mut c = pool.take().await?;
    let setup = format!(
        "SET ROWCOUNT {}; SET LOCK_TIMEOUT {}; SET XACT_ABORT ON; BEGIN TRANSACTION;",
        u64::from(limits.row_limit) + 1,
        limits.timeout_ms
    );
    simple(&mut c, &setup).await?;
    let result = read_rows(&mut c, sql, binds).await;
    // Whatever happened, nothing is kept.
    let cleanup = simple(
        &mut c,
        "IF @@TRANCOUNT > 0 ROLLBACK TRANSACTION; SET ROWCOUNT 0;",
    )
    .await;
    let (columns, mut rows) = result?;
    cleanup?;
    pool.give_back(c);
    let truncated = rows.len() > limits.row_limit as usize;
    rows.truncate(limits.row_limit as usize);
    Ok(RawResult {
        columns,
        rows,
        truncated,
    })
}

pub(crate) async fn run_write(
    pool: &MssqlPool,
    sql: &str,
    binds: &[Bind],
    limits: Limits,
    opts: WriteOptions,
) -> Result<WriteResult, SqlError> {
    let mut c = pool.take().await?;
    let setup = format!(
        "SET LOCK_TIMEOUT {}; SET XACT_ABORT ON; BEGIN TRANSACTION;",
        limits.timeout_ms
    );
    simple(&mut c, &setup).await?;
    let done = query(sql, binds).execute(&mut c).await.map_err(db_err);
    let affected = match done {
        Ok(r) => r.total(),
        Err(e) => {
            simple(&mut c, "IF @@TRANCOUNT > 0 ROLLBACK TRANSACTION;").await?;
            pool.give_back(c);
            return Err(e);
        }
    };
    let max = u64::from(opts.max_affected);
    let keep = affected <= max && !opts.dry_run;
    simple(
        &mut c,
        if keep {
            "COMMIT TRANSACTION;"
        } else {
            "IF @@TRANCOUNT > 0 ROLLBACK TRANSACTION;"
        },
    )
    .await?;
    pool.give_back(c);
    if affected > max {
        return Err(SqlError::TooManyRows {
            affected,
            max: opts.max_affected,
            at_least: false,
        });
    }
    Ok(WriteResult {
        affected,
        committed: keep,
        ..WriteResult::default()
    })
}

/// Server version, login and what lets it write.
pub(crate) async fn probe(pool: &MssqlPool) -> Result<crate::pool::ConnectionReport, SqlError> {
    let raw = run_read_only(
        pool,
        "SELECT CAST(SERVERPROPERTY('ProductVersion') AS nvarchar(128)), \
           CAST(SERVERPROPERTY('Edition') AS nvarchar(128)), SUSER_SNAME(), \
           IS_SRVROLEMEMBER('sysadmin'), IS_ROLEMEMBER('db_owner'), IS_ROLEMEMBER('db_datawriter'), \
           IS_ROLEMEMBER('db_ddladmin'), \
           HAS_PERMS_BY_NAME(DB_NAME(), 'DATABASE', 'INSERT'), HAS_PERMS_BY_NAME(DB_NAME(), 'DATABASE', 'UPDATE'), \
           HAS_PERMS_BY_NAME(DB_NAME(), 'DATABASE', 'DELETE'), HAS_PERMS_BY_NAME(DB_NAME(), 'DATABASE', 'ALTER')",
        &[],
        Limits {
            row_limit: 1,
            timeout_ms: 10_000,
        },
    )
    .await?;
    let row = raw.rows.first().cloned().unwrap_or_default();
    let text = |i: usize| row.get(i).and_then(Value::as_str).map(str::to_owned);
    let yes = |i: usize| row.get(i).and_then(Value::as_i64) == Some(1);
    let mut r = crate::pool::ConnectionReport {
        server_version: text(0).map(|v| match text(1) {
            Some(e) => format!("SQL Server {v} ({e})"),
            None => format!("SQL Server {v}"),
        }),
        user: text(2),
        ..Default::default()
    };
    for (i, what) in [
        (3, "sysadmin"),
        (4, "db_owner"),
        (5, "db_datawriter"),
        (6, "db_ddladmin"),
        (7, "INSERT on the database"),
        (8, "UPDATE on the database"),
        (9, "DELETE on the database"),
        (10, "ALTER on the database"),
    ] {
        if yes(i) {
            r.write_privileges.push(what.into());
        }
    }
    r.can_write = Some(!r.write_privileges.is_empty());
    if r.can_write == Some(true) {
        r.warnings.push(
            "SQL Server has no read-only transactions: mcp-forge checks every statement and rolls \
             back every read, but a login with only db_datareader is safer."
                .into(),
        );
    }
    Ok(r)
}

#[cfg(test)]
#[path = "../tests/unit/mssql.rs"]
mod tests;
