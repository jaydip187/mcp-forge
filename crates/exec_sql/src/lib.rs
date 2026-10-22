//! SQL connections, statement guards and the read-only query executor.
//!
//! Read-only enforcement is layered; each layer works on its own:
//! 1. the UI recommends a SELECT-only role and [`SqlExecutor::test_connection`]
//!    reports whether the configured role can write;
//! 2. the session is read-only with a server-side timeout ([`session`]);
//! 3. the statement must be a single read-only query, checked on the full
//!    AST ([`guard::check_read_only`]);
//! 4. functions that write or escape the database are denied ([`guard`]);
//! 5. results are capped by row limit, timeout and cell length.

mod explore;
pub mod guard;
pub mod session;
pub mod value;

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use forge_core::{
    check_args, ColumnMask, DbConnection, Driver, SecretResolver, SqlMode, SqlSpec,
    ValidationErrors, PATTERN_ARG, SQL_ARG, TABLE_ARG,
};
use serde::Serialize;
use serde_json::{json, Map, Value};
use sqlx::mysql::{MySqlConnectOptions, MySqlPoolOptions};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Connection, Executor, Row};
use uuid::Uuid;

pub use session::{run_read_only, Bind, DbPool, Limits, RawResult};

/// Replacement value for masked columns.
pub const MASKED: &str = "[masked]";

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SqlError {
    #[error("{0}")]
    InvalidArgs(ValidationErrors),
    /// The statement failed the read-only check (layers 3–4).
    #[error("query rejected: {0}")]
    Rejected(String),
    /// Missing connection, unresolvable secret, malformed DSN.
    #[error("{0}")]
    Config(String),
    #[error("could not connect to the database: {0}")]
    Connect(String),
    #[error("the query did not finish within {0} ms")]
    Timeout(u32),
    #[error("database error: {0}")]
    Database(String),
}

impl SqlError {
    fn connect(e: sqlx::Error) -> Self {
        SqlError::Connect(e.to_string())
    }

    fn redact(self, r: &Redactor) -> Self {
        match self {
            SqlError::Rejected(m) => SqlError::Rejected(r.apply(&m)),
            SqlError::Config(m) => SqlError::Config(r.apply(&m)),
            SqlError::Connect(m) => SqlError::Connect(r.apply(&m)),
            SqlError::Database(m) => SqlError::Database(r.apply(&m)),
            other => other,
        }
    }
}

impl From<sqlx::Error> for SqlError {
    fn from(e: sqlx::Error) -> Self {
        if let sqlx::Error::Database(d) = &e {
            // Postgres query_canceled; MySQL 3024 max_execution_time exceeded.
            if d.code().as_deref() == Some("57014")
                || d.try_downcast_ref::<sqlx::mysql::MySqlDatabaseError>()
                    .is_some_and(|m| m.number() == 3024)
            {
                return SqlError::Timeout(0);
            }
            return SqlError::Database(d.message().to_owned());
        }
        match e {
            sqlx::Error::PoolTimedOut
            | sqlx::Error::Io(_)
            | sqlx::Error::Tls(_)
            | sqlx::Error::Configuration(_) => SqlError::Connect(e.to_string()),
            other => SqlError::Database(other.to_string()),
        }
    }
}

/// Strips the DSN and its password from messages before they leave the executor.
struct Redactor(Vec<String>);

impl Redactor {
    fn new(dsn: &str) -> Self {
        let mut secrets = vec![dsn.to_owned()];
        if let Ok(u) = url::Url::parse(dsn) {
            if let Some(pw) = u.password() {
                secrets.push(pw.to_owned());
                if let Ok(decoded) = urldecode(pw) {
                    secrets.push(decoded);
                }
            }
        }
        // libpq key=value form: `password=...`.
        for part in dsn.split_whitespace() {
            if let Some(pw) = part.strip_prefix("password=") {
                secrets.push(pw.trim_matches('\'').to_owned());
            }
        }
        secrets.retain(|s| s.len() >= 3);
        secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
        Self(secrets)
    }

    fn apply(&self, msg: &str) -> String {
        self.0
            .iter()
            .fold(msg.to_owned(), |m, s| m.replace(s.as_str(), "***"))
    }
}

fn urldecode(s: &str) -> Result<String, std::string::FromUtf8Error> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out)
}

/// What a query tool returned, plus what was sent (for the test console).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SqlOutcome {
    /// Statement as executed (positional placeholders).
    pub sql: String,
    /// Bound values, in bind order.
    pub params: Vec<Value>,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub truncated: bool,
    /// Columns whose values were replaced by [`MASKED`].
    pub masked_columns: Vec<String>,
    pub elapsed_ms: u64,
}

impl SqlOutcome {
    /// The agent-facing result: `{columns, rows, truncated}`.
    pub fn result_json(&self) -> Value {
        json!({ "columns": self.columns, "rows": self.rows, "truncated": self.truncated })
    }
}

/// Result of "Test connection".
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ConnectionReport {
    pub ok: bool,
    pub server_version: Option<String>,
    pub user: Option<String>,
    /// Whether the configured role/file can modify data (`None` if unknown).
    pub can_write: Option<bool>,
    /// What gives it write access, e.g. "superuser" or "INSERT on shop.*".
    pub write_privileges: Vec<String>,
    pub warnings: Vec<String>,
    pub error: Option<String>,
    pub elapsed_ms: u64,
}

/// Save-time checks for a query tool: every `:name` has a param and vice
/// versa, and the statement passes the read-only guard.
pub fn check_query_tool(spec: &SqlSpec, driver: Driver) -> Result<(), ValidationErrors> {
    let SqlMode::Query { sql, params } = &spec.mode else {
        // Exploration tools carry no SQL of their own.
        return Ok(());
    };
    let mut errs = ValidationErrors::default();
    let bound = match guard::bind_placeholders(sql, driver) {
        Ok(b) => b,
        Err(e) => return Err(ValidationErrors::single("kind.mode.sql", e)),
    };
    for name in &bound.names {
        if !params.iter().any(|p| &p.name == name) {
            errs.push(
                "kind.mode.params",
                format!("`:{name}` in the SQL has no matching param"),
            );
        }
    }
    for (i, p) in params.iter().enumerate() {
        if !bound.names.contains(&p.name) {
            errs.push(
                format!("kind.mode.params[{i}].name"),
                format!(
                    "param `{}` is not used in the SQL (write `:{}`)",
                    p.name, p.name
                ),
            );
        }
    }
    if let Err(e) = guard::check_read_only(&bound.sql, driver) {
        errs.push("kind.mode.sql", e);
    }
    errs.into_result()
}

/// Turns validated arguments into positional binds.
fn binds_for(
    spec: &SqlSpec,
    names: &[String],
    args: &Map<String, Value>,
) -> Result<Vec<Bind>, SqlError> {
    names
        .iter()
        .map(|name| {
            let p = spec
                .params()
                .iter()
                .find(|p| &p.name == name)
                .ok_or_else(|| SqlError::Config(format!("`:{name}` has no matching param")))?;
            let v = args.get(name).filter(|v| !v.is_null());
            let ty: &'static str = match p.json_type() {
                "integer" => "integer",
                "number" => "number",
                "boolean" => "boolean",
                _ => "string",
            };
            Ok(match (ty, v) {
                (t, None) => Bind::Null(t),
                ("integer", Some(v)) => Bind::Int(
                    v.as_i64()
                        .or_else(|| v.as_f64().map(|f| f as i64))
                        .unwrap_or_default(),
                ),
                ("number", Some(v)) => Bind::Float(v.as_f64().unwrap_or_default()),
                ("boolean", Some(v)) => Bind::Bool(v.as_bool().unwrap_or_default()),
                (_, Some(Value::String(s))) => Bind::Text(s.clone()),
                (_, Some(other)) => Bind::Text(other.to_string()),
            })
        })
        .collect()
}

/// Builds the outcome of a query and masks its result columns.
fn finish(
    conn: &DbConnection,
    sql: String,
    binds: &[Bind],
    tables: &[String],
    raw: RawResult,
) -> SqlOutcome {
    let mut rows = raw.rows;
    let masked_columns = apply_masks(&conn.column_masks, tables, &raw.columns, &mut rows);
    SqlOutcome {
        sql,
        params: binds.iter().map(Bind::to_json).collect(),
        columns: raw.columns,
        rows,
        truncated: raw.truncated,
        masked_columns,
        elapsed_ms: 0,
    }
}

fn allowed_list(conn: &DbConnection) -> String {
    if conn.table_allowlist.is_empty() {
        "any non-system table".into()
    } else {
        conn.table_allowlist.join(", ")
    }
}

/// System catalogs are never available; others need the allowlist.
fn check_table_access(conn: &DbConnection, table: &str) -> Result<(), SqlError> {
    if guard::is_system_table(table, conn.driver) {
        return Err(SqlError::Rejected(format!(
            "`{table}` is a system catalog; use list_tables and describe_table to explore the schema"
        )));
    }
    if !guard::table_allowed(table, &conn.table_allowlist, conn.driver) {
        return Err(SqlError::Rejected(format!(
            "table `{table}` isn't available through this connection; allowed: {}",
            allowed_list(conn)
        )));
    }
    Ok(())
}

/// The checks agent-written SQL gets on top of the read-only guard: no
/// placeholders, only allowed tables, no way around column masks.
pub fn check_agent_sql(conn: &DbConnection, sql: &str) -> Result<guard::CheckedQuery, SqlError> {
    match guard::bind_placeholders(sql, conn.driver) {
        Ok(b) if b.names.is_empty() => {}
        _ => {
            return Err(SqlError::Rejected(
                "placeholders aren't supported here; write values inline".into(),
            ))
        }
    }
    let checked = guard::check_read_only(sql, conn.driver).map_err(SqlError::Rejected)?;
    for t in &checked.tables {
        check_table_access(conn, t)?;
    }
    guard::check_masks(&checked, &conn.column_masks).map_err(SqlError::Rejected)?;
    Ok(checked)
}

/// Replaces masked columns' values; returns the masked column names.
fn apply_masks(
    masks: &[ColumnMask],
    tables: &[String],
    columns: &[String],
    rows: &mut [Vec<Value>],
) -> Vec<String> {
    let hit: Vec<usize> = columns
        .iter()
        .enumerate()
        .filter(|(_, c)| masks.iter().any(|m| m.applies(c, tables)))
        .map(|(i, _)| i)
        .collect();
    for row in rows.iter_mut() {
        for &i in &hit {
            if let Some(v) = row.get_mut(i) {
                if !v.is_null() {
                    *v = Value::String(MASKED.into());
                }
            }
        }
    }
    hit.iter().map(|&i| columns[i].clone()).collect()
}

/// Runs SQL tools. Keeps one small pool per connection, rebuilt when the
/// connection's settings or DSN change.
#[derive(Clone)]
pub struct SqlExecutor {
    secrets: Arc<dyn SecretResolver>,
    pools: Arc<Mutex<HashMap<Uuid, (u64, DbPool)>>>,
}

impl SqlExecutor {
    pub fn new(secrets: Arc<dyn SecretResolver>) -> Self {
        Self {
            secrets,
            pools: Arc::default(),
        }
    }

    async fn resolve_dsn(&self, conn: &DbConnection) -> Result<String, SqlError> {
        let secrets = self.secrets.clone();
        let r = conn.dsn_ref.clone();
        tokio::task::spawn_blocking(move || secrets.resolve(&r))
            .await
            .map_err(|e| SqlError::Config(format!("secret lookup failed: {e}")))?
            .map_err(|e| SqlError::Config(format!("connection string unavailable: {e}")))
    }

    async fn pool(&self, conn: &DbConnection, dsn: &str) -> Result<DbPool, SqlError> {
        let mut h = DefaultHasher::new();
        (conn.driver, dsn, conn.timeout_ms).hash(&mut h);
        let key = h.finish();
        let stale = {
            let pools = self.pools.lock().expect("pool map poisoned");
            match pools.get(&conn.id) {
                Some((k, p)) if *k == key => return Ok(p.clone()),
                Some((_, p)) => Some(p.clone()),
                None => None,
            }
        };
        if let Some(old) = stale {
            old.close().await;
        }
        let pool = build_pool(conn, dsn)?;
        self.pools
            .lock()
            .expect("pool map poisoned")
            .insert(conn.id, (key, pool.clone()));
        Ok(pool)
    }

    /// The (cached) pool for a connection, opened with the read-only
    /// connection settings. Queries on it should still go through
    /// [`run_read_only`]; tests use it to exercise that layer alone.
    pub async fn pool_for(&self, conn: &DbConnection) -> Result<DbPool, SqlError> {
        let dsn = self.resolve_dsn(conn).await?;
        self.pool(conn, &dsn)
            .await
            .map_err(|e| e.redact(&Redactor::new(&dsn)))
    }

    /// Drops the cached pool of a deleted or edited connection.
    pub async fn forget(&self, connection_id: Uuid) {
        let old = self
            .pools
            .lock()
            .expect("pool map poisoned")
            .remove(&connection_id);
        if let Some((_, p)) = old {
            p.close().await;
        }
    }

    /// Runs `sql` read-only on the connection's pool, with its limits.
    /// Errors are redacted; timeouts report the connection's limit.
    async fn execute(
        &self,
        conn: &DbConnection,
        sql: &str,
        binds: &[Bind],
        row_limit: u32,
    ) -> Result<RawResult, SqlError> {
        let dsn = self.resolve_dsn(conn).await?;
        let redactor = Redactor::new(&dsn);
        let limits = Limits {
            row_limit,
            timeout_ms: conn.timeout_ms,
        };
        async {
            let pool = self.pool(conn, &dsn).await?;
            run_read_only(&pool, sql, binds, limits).await
        }
        .await
        .map_err(|e| match e {
            SqlError::Timeout(_) => SqlError::Timeout(conn.timeout_ms),
            other => other.redact(&redactor),
        })
    }

    /// Runs a SQL tool with the agent's arguments: a curated query or one of
    /// the exploration tools.
    pub async fn run_query(
        &self,
        conn: &DbConnection,
        spec: &SqlSpec,
        args: &Map<String, Value>,
    ) -> Result<SqlOutcome, SqlError> {
        let derived = spec.derive_input();
        check_args(&derived.bindings, &derived.schema, args).map_err(SqlError::InvalidArgs)?;
        let started = Instant::now();
        let text = |k: &str| {
            args.get(k)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
        };
        let mut out = match &spec.mode {
            SqlMode::Query { sql, .. } => {
                let bound =
                    guard::bind_placeholders(sql, conn.driver).map_err(SqlError::Rejected)?;
                let checked =
                    guard::check_read_only(&bound.sql, conn.driver).map_err(SqlError::Rejected)?;
                let binds = binds_for(spec, &bound.binds, args)?;
                let raw = self
                    .execute(conn, &bound.sql, &binds, conn.row_limit)
                    .await?;
                finish(conn, bound.sql, &binds, &checked.tables, raw)
            }
            SqlMode::RunSql => {
                let sql = text(SQL_ARG).ok_or_else(|| {
                    SqlError::InvalidArgs(ValidationErrors::single(SQL_ARG, "must not be empty"))
                })?;
                let checked = check_agent_sql(conn, sql)?;
                let raw = self.execute(conn, sql, &[], conn.row_limit).await?;
                finish(conn, sql.to_owned(), &[], &checked.tables, raw)
            }
            SqlMode::ListTables => {
                let sql = explore::list_tables_sql(conn.driver);
                let raw = self.execute(conn, sql, &[], explore::MAX_ROWS).await?;
                explore::list_tables(conn, sql, raw, text(PATTERN_ARG))
            }
            SqlMode::DescribeTable => {
                let table = text(TABLE_ARG).ok_or_else(|| {
                    SqlError::InvalidArgs(ValidationErrors::single(TABLE_ARG, "must not be empty"))
                })?;
                check_table_access(conn, table)?;
                let (sql, binds) = explore::describe_table_sql(conn.driver, table);
                let raw = self.execute(conn, &sql, &binds, explore::MAX_ROWS).await?;
                explore::describe_table(conn, table, sql, &binds, raw)?
            }
        };
        out.elapsed_ms = started.elapsed().as_millis() as u64;
        Ok(out)
    }

    /// Connects, reads the server version and checks whether the role can
    /// write. Never fails: problems are reported in the result.
    pub async fn test_connection(&self, conn: &DbConnection) -> ConnectionReport {
        let started = Instant::now();
        let mut report = match self.resolve_dsn(conn).await {
            Err(e) => ConnectionReport {
                error: Some(e.to_string()),
                ..Default::default()
            },
            Ok(dsn) => {
                let redactor = Redactor::new(&dsn);
                let probe = async {
                    let pool = self.pool(conn, &dsn).await?;
                    let budget = Duration::from_millis(u64::from(conn.timeout_ms));
                    match tokio::time::timeout(budget + Duration::from_secs(2), probe(&pool, &dsn))
                        .await
                    {
                        Ok(r) => r,
                        Err(_) => Err(SqlError::Timeout(conn.timeout_ms)),
                    }
                };
                match probe.await {
                    Ok(r) => r,
                    Err(e) => ConnectionReport {
                        error: Some(e.redact(&redactor).to_string()),
                        ..Default::default()
                    },
                }
            }
        };
        report.ok = report.error.is_none();
        if report.can_write == Some(true) {
            report.warnings.push(
                "This connection can modify data. mcp-forge still blocks writes (read-only \
                 session, statement check, function denylist), but a SELECT-only role is \
                 recommended."
                    .into(),
            );
        }
        report.elapsed_ms = started.elapsed().as_millis() as u64;
        report
    }
}

fn build_pool(conn: &DbConnection, dsn: &str) -> Result<DbPool, SqlError> {
    let bad = |e: sqlx::Error| SqlError::Config(format!("invalid connection string: {e}"));
    let acquire = Duration::from_millis(u64::from(conn.timeout_ms)).max(Duration::from_secs(5));
    let idle = Duration::from_secs(300);
    Ok(match conn.driver {
        Driver::Sqlite => {
            let s = if dsn.starts_with("sqlite:") {
                dsn.to_owned()
            } else {
                format!("sqlite://{dsn}")
            };
            let opts = SqliteConnectOptions::from_str(&s)
                .map_err(bad)?
                .read_only(true)
                .create_if_missing(false)
                .pragma("query_only", "ON")
                .busy_timeout(Duration::from_millis(u64::from(conn.timeout_ms)));
            DbPool::Sqlite(
                SqlitePoolOptions::new()
                    .max_connections(4)
                    .acquire_timeout(acquire)
                    .idle_timeout(idle)
                    .connect_lazy_with(opts),
            )
        }
        Driver::Postgres => {
            require_scheme(dsn, &["postgres://", "postgresql://"])?;
            let opts = PgConnectOptions::from_str(dsn)
                .map_err(bad)?
                .application_name("mcp-forge")
                .options([("default_transaction_read_only", "on")]);
            DbPool::Postgres(
                PgPoolOptions::new()
                    .max_connections(4)
                    .acquire_timeout(acquire)
                    .idle_timeout(idle)
                    .connect_lazy_with(opts),
            )
        }
        Driver::Mysql => {
            require_scheme(dsn, &["mysql://", "mariadb://"])?;
            let opts = MySqlConnectOptions::from_str(dsn).map_err(bad)?;
            DbPool::Mysql(
                MySqlPoolOptions::new()
                    .max_connections(4)
                    .acquire_timeout(acquire)
                    .idle_timeout(idle)
                    .connect_lazy_with(opts),
            )
        }
    })
}

/// Postgres/MySQL DSNs must be URLs; anything else (say, a SQLite path)
/// would otherwise fall back to default host settings and connect elsewhere.
fn require_scheme(dsn: &str, schemes: &[&str]) -> Result<(), SqlError> {
    if schemes
        .iter()
        .any(|s| dsn.to_ascii_lowercase().starts_with(s))
    {
        Ok(())
    } else {
        Err(SqlError::Config(format!(
            "the connection string must start with {}",
            schemes.join(" or ")
        )))
    }
}

async fn probe(pool: &DbPool, dsn: &str) -> Result<ConnectionReport, SqlError> {
    let mut r = ConnectionReport::default();
    match pool {
        DbPool::Postgres(p) => {
            let mut tx = p
                .begin_with("BEGIN READ ONLY")
                .await
                .map_err(SqlError::connect)?;
            let row = sqlx::query(
                "SELECT current_user::text, current_setting('server_version'), r.rolsuper, \
                   has_database_privilege(current_database(), 'CREATE'), \
                   (SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
                     WHERE c.relkind IN ('r', 'p', 'v', 'm', 'f') \
                       AND n.nspname NOT IN ('pg_catalog', 'information_schema') \
                       AND n.nspname NOT LIKE 'pg_toast%' \
                       AND (has_table_privilege(c.oid, 'INSERT') OR has_table_privilege(c.oid, 'UPDATE') \
                         OR has_table_privilege(c.oid, 'DELETE') OR has_table_privilege(c.oid, 'TRUNCATE'))), \
                   (SELECT count(*) FROM pg_namespace n \
                     WHERE n.nspname NOT IN ('pg_catalog', 'information_schema') \
                       AND n.nspname NOT LIKE 'pg\\_%' AND has_schema_privilege(n.oid, 'CREATE')) \
                 FROM pg_roles r WHERE r.rolname = current_user",
            )
            .fetch_one(&mut *tx)
            .await?;
            tx.rollback().await?;
            r.user = row.try_get(0).ok();
            r.server_version = row
                .try_get::<String, _>(1)
                .ok()
                .map(|v| format!("PostgreSQL {v}"));
            let superuser: bool = row.try_get(2).unwrap_or(false);
            let db_create: bool = row.try_get(3).unwrap_or(false);
            let tables: i64 = row.try_get(4).unwrap_or(0);
            let schemas: i64 = row.try_get(5).unwrap_or(0);
            if superuser {
                r.write_privileges.push("superuser".into());
            }
            if tables > 0 {
                r.write_privileges.push(format!(
                    "INSERT/UPDATE/DELETE/TRUNCATE on {tables} table(s)"
                ));
            }
            if schemas > 0 {
                r.write_privileges
                    .push(format!("CREATE in {schemas} schema(s)"));
            }
            if db_create {
                r.write_privileges.push("CREATE on the database".into());
            }
            r.can_write = Some(!r.write_privileges.is_empty());
        }
        DbPool::Mysql(p) => {
            let mut conn = p.acquire().await.map_err(SqlError::connect)?;
            let mut tx = conn.begin_with("START TRANSACTION READ ONLY").await?;
            let row = sqlx::query("SELECT CAST(CURRENT_USER() AS CHAR), CAST(VERSION() AS CHAR)")
                .fetch_one(&mut *tx)
                .await?;
            r.user = row.try_get(0).ok();
            r.server_version = row
                .try_get::<String, _>(1)
                .ok()
                .map(|v| format!("MySQL {v}"));
            let grants = (&mut *tx).fetch_all("SHOW GRANTS").await?;
            tx.rollback().await?;
            let lines: Vec<String> = grants
                .iter()
                .filter_map(|g| g.try_get::<String, _>(0).ok())
                .collect();
            let (privs, roles) = mysql_write_grants(&lines);
            r.write_privileges = privs;
            if roles {
                r.warnings.push(
                    "The user has roles; their privileges are not included in this check.".into(),
                );
            }
            r.can_write = Some(!r.write_privileges.is_empty());
        }
        DbPool::Sqlite(p) => {
            let v: String = sqlx::query_scalar("SELECT sqlite_version()")
                .fetch_one(p)
                .await?;
            r.server_version = Some(format!("SQLite {v}"));
            let path = dsn
                .trim_start_matches("sqlite://")
                .trim_start_matches("sqlite:");
            let path = path.split('?').next().unwrap_or(path);
            match std::fs::metadata(path) {
                Ok(m) if !m.permissions().readonly() => {
                    r.write_privileges
                        .push("the database file is writable by mcp-forge's OS user".into());
                    r.can_write = Some(true);
                }
                Ok(_) => r.can_write = Some(false),
                Err(_) => r.can_write = None,
            }
        }
    }
    Ok(r)
}

/// Write-capable privileges in `SHOW GRANTS` output, and whether roles are granted.
fn mysql_write_grants(lines: &[String]) -> (Vec<String>, bool) {
    const WRITE: &[&str] = &[
        "ALL",
        "INSERT",
        "UPDATE",
        "DELETE",
        "CREATE",
        "DROP",
        "ALTER",
        "INDEX",
        "TRIGGER",
        "EVENT",
        "FILE",
        "SUPER",
        "EXECUTE",
        "CREATE ROUTINE",
        "ALTER ROUTINE",
    ];
    let mut out = Vec::new();
    let mut roles = false;
    for line in lines {
        let Some(rest) = line.strip_prefix("GRANT ") else {
            continue;
        };
        let Some((privs, target)) = rest.split_once(" ON ") else {
            roles = true; // `GRANT role TO user`
            continue;
        };
        let on = target.split(" TO ").next().unwrap_or(target).trim();
        for p in privs.split(',').map(str::trim) {
            let up = p.to_uppercase();
            let base = up.split('(').next().unwrap_or(&up).trim().to_owned();
            if WRITE.contains(&base.as_str())
                || base.starts_with("ALL")
                || (base.ends_with("_ADMIN") && base != "ROLE_ADMIN")
            {
                out.push(format!("{p} ON {on}"));
            }
        }
    }
    (out, roles)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(sql: &str, params: Value) -> SqlSpec {
        serde_json::from_value(json!({
            "connection_id": Uuid::nil(),
            "mode": { "type": "query", "sql": sql, "params": params }
        }))
        .unwrap()
    }

    #[test]
    fn save_time_check() {
        let s = spec(
            "SELECT * FROM users WHERE id = :id",
            json!([{ "name": "id", "schema": { "type": "integer" } }]),
        );
        check_query_tool(&s, Driver::Postgres).unwrap();

        let s = spec(
            "SELECT * FROM users WHERE id = :id",
            json!([{ "name": "other" }]),
        );
        let e = check_query_tool(&s, Driver::Sqlite).unwrap_err();
        assert!(
            e.has("kind.mode.params") && e.has("kind.mode.params[0].name"),
            "{e}"
        );

        let s = spec("DELETE FROM users", json!([]));
        let e = check_query_tool(&s, Driver::Mysql).unwrap_err();
        assert!(e.has("kind.mode.sql"), "{e}");
    }

    #[test]
    fn binds_follow_declared_types() {
        let s = spec(
            "SELECT :i, :n, :b, :s, :missing",
            json!([
                { "name": "i", "schema": { "type": "integer" } },
                { "name": "n", "schema": { "type": "number" } },
                { "name": "b", "schema": { "type": "boolean" } },
                { "name": "s" },
                { "name": "missing", "schema": { "type": "integer" } }
            ]),
        );
        let args = json!({ "i": 3.0, "n": 1.5, "b": true, "s": "x" });
        let binds = binds_for(
            &s,
            &["i", "n", "b", "s", "missing"].map(String::from),
            args.as_object().unwrap(),
        )
        .unwrap();
        assert_eq!(
            binds,
            [
                Bind::Int(3),
                Bind::Float(1.5),
                Bind::Bool(true),
                Bind::Text("x".into()),
                Bind::Null("integer")
            ]
        );
    }

    #[test]
    fn masks_replace_values() {
        let masks = [
            ColumnMask::parse("users.password_hash").unwrap(),
            ColumnMask::parse("*.email").unwrap(),
        ];
        let cols = ["id", "email", "password_hash"].map(String::from);
        let mut rows = vec![
            vec![json!(1), json!("a@x.io"), json!("h")],
            vec![json!(2), Value::Null, json!("h2")],
        ];
        let hit = apply_masks(&masks, &["users".into()], &cols, &mut rows);
        assert_eq!(hit, ["email", "password_hash"]);
        assert_eq!(rows[0], [json!(1), json!(MASKED), json!(MASKED)]);
        assert_eq!(rows[1], [json!(2), Value::Null, json!(MASKED)]);
        // Table-scoped masks don't apply to other tables.
        let mut rows = vec![vec![json!(1), json!("e"), json!("h")]];
        let hit = apply_masks(&masks, &["orders".into()], &cols, &mut rows);
        assert_eq!(hit, ["email"]);
    }

    #[test]
    fn redacts_dsn_and_password() {
        let r = Redactor::new("postgres://app:s3cr%40t@db.internal:5432/shop");
        let m = r.apply("failed for postgres://app:s3cr%40t@db.internal:5432/shop (pw s3cr@t)");
        assert!(!m.contains("s3cr"), "{m}");
        let r = Redactor::new("host=db user=app password=hunter22 dbname=shop");
        assert!(!r.apply("bad password hunter22").contains("hunter22"));
    }

    #[test]
    fn network_drivers_need_url_dsns() {
        let conn: DbConnection = serde_json::from_value(json!({
            "id": Uuid::nil(), "project_id": Uuid::nil(), "name": "db", "driver": "postgres",
            "dsn_ref": "env:DB", "access_mode": "read_only", "row_limit": 1, "timeout_ms": 1000,
            "max_affected_rows": 1, "allow_ddl": false, "table_allowlist": [], "column_masks": [],
            "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z"
        }))
        .unwrap();
        let e = build_pool(&conn, r"C:\data\shop.db").unwrap_err();
        assert!(e.to_string().contains("postgres://"), "{e}");
    }

    #[test]
    fn mysql_grant_parsing() {
        let (p, roles) = mysql_write_grants(&[
            "GRANT USAGE ON *.* TO `ro`@`%`".into(),
            "GRANT SELECT ON `shop`.* TO `ro`@`%`".into(),
        ]);
        assert!(p.is_empty() && !roles);
        let (p, roles) = mysql_write_grants(&[
            "GRANT SELECT, INSERT, UPDATE ON `shop`.* TO `app`@`%`".into(),
            "GRANT `writer`@`%` TO `app`@`%`".into(),
        ]);
        assert_eq!(p, ["INSERT ON `shop`.*", "UPDATE ON `shop`.*"]);
        assert!(roles);
        let (p, _) = mysql_write_grants(&[
            "GRANT ALL PRIVILEGES ON *.* TO `root`@`%` WITH GRANT OPTION".into(),
        ]);
        assert_eq!(p.len(), 1);
    }
}
