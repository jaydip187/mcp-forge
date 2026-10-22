//! The pooled executor that runs SQL tools.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use forge_core::{
    check_args, AuditOutcome, DbConnection, Driver, SecretResolver, SqlMode, SqlSpec,
    ValidationErrors, PATTERN_ARG, SQL_ARG, TABLE_ARG,
};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::{
    error::Redactor, error::SqlError, explore, guard, outcome::AuditDraft, outcome::SqlOutcome,
    outcome::SqlRun, outcome::WriteInfo, outcome::DRY_RUN_ARG, policy::apply_masks,
    policy::binds_for, policy::check_agent_sql, policy::check_table_access, policy::finish,
    policy::gate, policy::tool_input, pool::build_pool, pool::probe, pool::ConnectionReport,
    run_read_only, session, Bind, DbPool, Limits, RawResult, StatementClass, WriteOptions,
};

/// Cached pools: (connection, writable) → (settings fingerprint, pool).
type PoolMap = HashMap<(Uuid, bool), (u64, DbPool)>;

/// Runs SQL tools. Keeps one small pool per connection, rebuilt when the
/// connection's settings or DSN change.
#[derive(Clone)]
pub struct SqlExecutor {
    secrets: Arc<dyn SecretResolver>,
    /// Keyed by connection and whether the pool may write (SQLite opens the
    /// file differently for writes).
    pools: Arc<Mutex<PoolMap>>,
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

    async fn pool(
        &self,
        conn: &DbConnection,
        dsn: &str,
        writable: bool,
    ) -> Result<DbPool, SqlError> {
        let mut h = DefaultHasher::new();
        (conn.driver, dsn, conn.timeout_ms).hash(&mut h);
        let key = h.finish();
        let stale = {
            let pools = self.pools.lock().expect("pool map poisoned");
            match pools.get(&(conn.id, writable)) {
                Some((k, p)) if *k == key => return Ok(p.clone()),
                Some((_, p)) => Some(p.clone()),
                None => None,
            }
        };
        if let Some(old) = stale {
            old.close().await;
        }
        let pool = build_pool(conn, dsn, writable)?;
        self.pools
            .lock()
            .expect("pool map poisoned")
            .insert((conn.id, writable), (key, pool.clone()));
        Ok(pool)
    }

    /// The (cached) pool for a connection, opened with the read-only
    /// connection settings. Queries on it should still go through
    /// [`run_read_only`]; tests use it to exercise that layer alone.
    pub async fn pool_for(&self, conn: &DbConnection) -> Result<DbPool, SqlError> {
        let dsn = self.resolve_dsn(conn).await?;
        self.pool(conn, &dsn, false)
            .await
            .map_err(|e| e.redact(&Redactor::new(&dsn)))
    }

    /// Drops the cached pool of a deleted or edited connection.
    pub async fn forget(&self, connection_id: Uuid) {
        let old: Vec<DbPool> = {
            let mut pools = self.pools.lock().expect("pool map poisoned");
            [false, true]
                .iter()
                .filter_map(|w| pools.remove(&(connection_id, *w)))
                .map(|(_, p)| p)
                .collect()
        };
        for p in old {
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
            let pool = self.pool(conn, &dsn, false).await?;
            run_read_only(&pool, sql, binds, limits).await
        }
        .await
        .map_err(|e| match e {
            SqlError::Timeout(_) => SqlError::Timeout(conn.timeout_ms),
            other => other.redact(&redactor),
        })
    }

    /// Runs a write in a read-write transaction and builds its audit record.
    async fn write(
        &self,
        conn: &DbConnection,
        sql: String,
        binds: Vec<Bind>,
        checked: &guard::CheckedQuery,
        dry_run: bool,
    ) -> SqlRun {
        if dry_run && conn.driver == Driver::Mysql && checked.class == StatementClass::Ddl {
            return SqlRun {
                result: Err(SqlError::Rejected(
                    "MySQL commits schema changes immediately, so dry_run isn't possible for them"
                        .into(),
                )),
                audit: None,
            };
        }
        let params: Vec<Value> = binds.iter().map(Bind::to_json).collect();
        let limits = Limits {
            row_limit: conn.row_limit,
            timeout_ms: conn.timeout_ms,
        };
        let opts = WriteOptions {
            max_affected: conn.max_affected_rows,
            dry_run,
            returning: checked.returning,
        };
        let result = async {
            let dsn = self.resolve_dsn(conn).await?;
            let redactor = Redactor::new(&dsn);
            async {
                let pool = self.pool(conn, &dsn, true).await?;
                session::run_write(&pool, &sql, &binds, limits, opts).await
            }
            .await
            .map_err(|e| match e {
                SqlError::Timeout(_) => SqlError::Timeout(conn.timeout_ms),
                other => other.redact(&redactor),
            })
        }
        .await;
        let mut audit = AuditDraft {
            sql: sql.clone(),
            params: params.clone(),
            class: checked.class,
            rows_affected: None,
            outcome: AuditOutcome::Failed,
            error: None,
        };
        let result = match result {
            Ok(w) => {
                audit.rows_affected = Some(w.affected);
                audit.outcome = if w.committed {
                    AuditOutcome::Committed
                } else {
                    AuditOutcome::DryRun
                };
                let mut rows = w.rows;
                let masked_columns =
                    apply_masks(&conn.column_masks, &checked.tables, &w.columns, &mut rows);
                Ok(SqlOutcome {
                    sql,
                    params,
                    columns: w.columns,
                    rows,
                    truncated: w.truncated,
                    masked_columns,
                    elapsed_ms: 0,
                    write: Some(WriteInfo {
                        class: checked.class.as_str().into(),
                        affected_rows: w.affected,
                        committed: w.committed,
                        dry_run,
                    }),
                })
            }
            Err(e) => {
                if let SqlError::TooManyRows { affected, .. } = &e {
                    audit.rows_affected = Some(*affected);
                    audit.outcome = AuditOutcome::RolledBack;
                }
                audit.error = Some(e.to_string());
                Err(e)
            }
        };
        SqlRun {
            result,
            audit: Some(audit),
        }
    }

    /// Runs a SQL tool with the agent's arguments and returns only the
    /// result (see [`SqlExecutor::run`] for the audit record).
    pub async fn run_query(
        &self,
        conn: &DbConnection,
        spec: &SqlSpec,
        args: &Map<String, Value>,
    ) -> Result<SqlOutcome, SqlError> {
        self.run(conn, spec, args).await.result
    }

    /// Runs a SQL tool: a saved query (read or write) or an exploration
    /// tool. Writes come back with an audit record, even when they fail.
    pub async fn run(
        &self,
        conn: &DbConnection,
        spec: &SqlSpec,
        args: &Map<String, Value>,
    ) -> SqlRun {
        let started = Instant::now();
        let mut run = match self.dispatch(conn, spec, args).await {
            Ok(r) => r,
            Err(e) => SqlRun {
                result: Err(e),
                audit: None,
            },
        };
        if let Ok(o) = &mut run.result {
            o.elapsed_ms = started.elapsed().as_millis() as u64;
        }
        run
    }

    async fn dispatch(
        &self,
        conn: &DbConnection,
        spec: &SqlSpec,
        args: &Map<String, Value>,
    ) -> Result<SqlRun, SqlError> {
        let derived = tool_input(spec, conn);
        check_args(&derived.bindings, &derived.schema, args).map_err(SqlError::InvalidArgs)?;
        let dry_run = args
            .get(DRY_RUN_ARG)
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let text = |k: &str| {
            args.get(k)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
        };
        let read = |o: SqlOutcome| SqlRun {
            result: Ok(o),
            audit: None,
        };
        Ok(match &spec.mode {
            SqlMode::Query { sql, .. } => {
                let bound =
                    guard::bind_placeholders(sql, conn.driver).map_err(SqlError::Rejected)?;
                let checked =
                    guard::check_statement(&bound.sql, conn.driver).map_err(SqlError::Rejected)?;
                gate(&checked, conn.access_mode, conn.allow_ddl, false)
                    .map_err(SqlError::Rejected)?;
                let binds = binds_for(spec, &bound.binds, args)?;
                if checked.class != StatementClass::Read {
                    return Ok(self.write(conn, bound.sql, binds, &checked, dry_run).await);
                }
                let raw = self
                    .execute(conn, &bound.sql, &binds, conn.row_limit)
                    .await?;
                read(finish(conn, bound.sql, &binds, &checked.tables, raw))
            }
            SqlMode::RunSql => {
                let sql = text(SQL_ARG).ok_or_else(|| {
                    SqlError::InvalidArgs(ValidationErrors::single(SQL_ARG, "must not be empty"))
                })?;
                let checked = check_agent_sql(conn, sql)?;
                if checked.class != StatementClass::Read {
                    return Ok(self
                        .write(conn, sql.to_owned(), Vec::new(), &checked, dry_run)
                        .await);
                }
                let raw = self.execute(conn, sql, &[], conn.row_limit).await?;
                read(finish(conn, sql.to_owned(), &[], &checked.tables, raw))
            }
            SqlMode::ListTables => {
                let sql = explore::list_tables_sql(conn.driver);
                let raw = self.execute(conn, sql, &[], explore::MAX_ROWS).await?;
                read(explore::list_tables(conn, sql, raw, text(PATTERN_ARG)))
            }
            SqlMode::DescribeTable => {
                let table = text(TABLE_ARG).ok_or_else(|| {
                    SqlError::InvalidArgs(ValidationErrors::single(TABLE_ARG, "must not be empty"))
                })?;
                check_table_access(conn, table)?;
                let (sql, binds) = explore::describe_table_sql(conn.driver, table);
                let raw = self.execute(conn, &sql, &binds, explore::MAX_ROWS).await?;
                read(explore::describe_table(conn, table, sql, &binds, raw)?)
            }
        })
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
                    let pool = self.pool(conn, &dsn, false).await?;
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
