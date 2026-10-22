//! Table allowlists and system catalogs.

use forge_core::Driver;

/// `(schema, name)` of a table reference, lower-cased. `a.b.c` → `(a.b, c)`.
fn split_table(table: &str) -> (Option<String>, String) {
    let t = table.to_lowercase();
    match t.rsplit_once('.') {
        Some((s, n)) => (Some(s.to_owned()), n.to_owned()),
        None => (None, t),
    }
}

/// The schema an unqualified name resolves to, when it's fixed.
fn default_schema(driver: Driver) -> Option<&'static str> {
    match driver {
        Driver::Postgres => Some("public"),
        Driver::Sqlite => Some("main"),
        // The connection's database; not known without asking the server.
        Driver::Mysql => None,
    }
}

/// System catalogs: never readable through `run_sql`, whatever the allowlist.
pub fn is_system_table(table: &str, driver: Driver) -> bool {
    const SYSTEM_SCHEMAS: &[&str] = &[
        "pg_catalog",
        "information_schema",
        "pg_toast",
        "mysql",
        "performance_schema",
        "sys",
    ];
    let (schema, name) = split_table(table);
    match schema {
        Some(s) => {
            let last = s.rsplit('.').next().unwrap_or(&s);
            SYSTEM_SCHEMAS.contains(&last)
                || (driver == Driver::Sqlite && name.starts_with("sqlite_"))
        }
        // Unqualified pg_* names resolve to pg_catalog first.
        None => {
            (driver == Driver::Postgres && name.starts_with("pg_"))
                || (driver == Driver::Sqlite && name.starts_with("sqlite_"))
        }
    }
}

/// Whether `table` (as written in a query or passed to `describe_table`) is
/// covered by the allowlist. Entries: `*`, `schema.*`, `schema.table` or
/// `table` (any schema). An empty allowlist allows every non-system table.
/// Unqualified references match `schema.…` entries only for the driver's
/// fixed default schema (`public`, `main`).
pub fn table_allowed(table: &str, allowlist: &[String], driver: Driver) -> bool {
    if allowlist.is_empty() {
        return true;
    }
    let (schema, name) = split_table(table);
    let schema = schema.or_else(|| default_schema(driver).map(str::to_owned));
    allowlist.iter().any(|entry| {
        let entry = entry.to_lowercase();
        if entry == "*" {
            return true;
        }
        match entry.rsplit_once('.') {
            None => entry == name,
            Some((es, en)) => schema.as_deref() == Some(es) && (en == "*" || en == name),
        }
    })
}
