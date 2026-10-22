//! Building pools per driver and probing a connection's privileges.

use std::str::FromStr;
use std::time::Duration;

use forge_core::{DbConnection, Driver};
use serde::Serialize;
use sqlx::mysql::{MySqlConnectOptions, MySqlPoolOptions};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Connection, Executor, Row};

use crate::{error::SqlError, DbPool};

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

/// Postgres and MySQL share one set of options (writes open a read-write
/// transaction explicitly); SQLite opens the file read-only unless `writable`.
pub(crate) fn build_pool(
    conn: &DbConnection,
    dsn: &str,
    writable: bool,
) -> Result<DbPool, SqlError> {
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
            let mut opts = SqliteConnectOptions::from_str(&s)
                .map_err(bad)?
                .read_only(!writable)
                .create_if_missing(false)
                .busy_timeout(Duration::from_millis(u64::from(conn.timeout_ms)));
            if !writable {
                opts = opts.pragma("query_only", "ON");
            }
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
pub(crate) fn require_scheme(dsn: &str, schemes: &[&str]) -> Result<(), SqlError> {
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

pub(crate) async fn probe(pool: &DbPool, dsn: &str) -> Result<ConnectionReport, SqlError> {
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
pub(crate) fn mysql_write_grants(lines: &[String]) -> (Vec<String>, bool) {
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
