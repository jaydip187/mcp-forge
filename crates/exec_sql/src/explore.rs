//! `list_tables` and `describe_table`: fixed catalog queries per driver,
//! filtered by the connection's allowlist and system-catalog rules. Results
//! use the same `{columns, rows, truncated}` shape as queries.

use forge_core::{DbConnection, Driver};
use serde_json::Value;

use crate::guard::{is_system_table, table_allowed};
use crate::{Bind, RawResult, SqlError, SqlOutcome};

/// Most catalog rows read for one call.
pub const MAX_ROWS: u32 = 2_000;

pub fn list_tables_sql(driver: Driver) -> &'static str {
    match driver {
        Driver::Postgres => {
            "SELECT table_schema::text, table_name::text, table_type::text \
             FROM information_schema.tables \
             WHERE table_schema NOT IN ('pg_catalog', 'information_schema') \
             ORDER BY 1, 2"
        }
        Driver::Mysql => {
            "SELECT CAST(table_schema AS CHAR), CAST(table_name AS CHAR), CAST(table_type AS CHAR) \
             FROM information_schema.tables WHERE table_schema = DATABASE() ORDER BY 2"
        }
        Driver::Sqlite => {
            "SELECT 'main', name, type FROM sqlite_master \
             WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' ORDER BY name"
        }
    }
}

fn text(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn truthy(v: Option<&Value>) -> bool {
    match v {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::String(s)) => {
            matches!(s.to_ascii_uppercase().as_str(), "YES" | "1" | "TRUE" | "T")
        }
        _ => false,
    }
}

/// Keeps allowed, non-system tables matching `pattern`.
pub fn list_tables(
    conn: &DbConnection,
    sql: &str,
    raw: RawResult,
    pattern: Option<&str>,
) -> SqlOutcome {
    let pattern = pattern.map(str::to_lowercase);
    let rows: Vec<Vec<Value>> = raw
        .rows
        .iter()
        .filter_map(|r| {
            let (schema, name) = (text(r.first()), text(r.get(1)));
            let qualified = format!("{schema}.{name}");
            let kind = match text(r.get(2)).to_lowercase().as_str() {
                "base table" | "table" => "table".to_owned(),
                other => other.to_owned(),
            };
            let visible = !is_system_table(&qualified, conn.driver)
                && table_allowed(&qualified, &conn.table_allowlist, conn.driver)
                && pattern
                    .as_ref()
                    .is_none_or(|p| name.to_lowercase().contains(p.as_str()));
            visible.then(|| {
                vec![
                    Value::String(schema),
                    Value::String(name),
                    Value::String(kind),
                ]
            })
        })
        .collect();
    SqlOutcome {
        sql: sql.to_owned(),
        params: Vec::new(),
        columns: ["schema", "name", "type"].map(String::from).to_vec(),
        rows,
        truncated: raw.truncated,
        masked_columns: Vec::new(),
        elapsed_ms: 0,
        write: None,
    }
}

/// `(schema, name)` as given; the name may be quoted-free `schema.table`.
fn split(table: &str) -> (Option<&str>, &str) {
    match table.rsplit_once('.') {
        Some((s, n)) => (Some(s), n),
        None => (None, table),
    }
}

/// Columns: schema, column, type, nullable, default, primary key.
pub fn describe_table_sql(driver: Driver, table: &str) -> (String, Vec<Bind>) {
    let (schema, name) = split(table);
    let schema_bind = schema.map_or(Bind::Null("string"), |s| Bind::Text(s.to_owned()));
    match driver {
        Driver::Postgres => (
            "SELECT c.table_schema::text, c.column_name::text, c.data_type::text, \
               c.is_nullable::text = 'YES', c.column_default::text, \
               EXISTS (SELECT 1 FROM information_schema.table_constraints tc \
                 JOIN information_schema.key_column_usage k \
                   ON k.constraint_name = tc.constraint_name AND k.constraint_schema = tc.constraint_schema \
                 WHERE tc.constraint_type = 'PRIMARY KEY' AND tc.table_schema = c.table_schema \
                   AND tc.table_name = c.table_name AND k.column_name = c.column_name) \
             FROM information_schema.columns c \
             WHERE lower(c.table_name) = lower($1) \
               AND ($2::text IS NULL OR lower(c.table_schema) = lower($2)) \
               AND c.table_schema NOT IN ('pg_catalog', 'information_schema') \
             ORDER BY c.table_schema, c.ordinal_position"
                .into(),
            vec![Bind::Text(name.to_owned()), schema_bind],
        ),
        Driver::Mysql => (
            "SELECT CAST(table_schema AS CHAR), CAST(column_name AS CHAR), CAST(column_type AS CHAR), \
               is_nullable = 'YES', CAST(column_default AS CHAR), column_key = 'PRI' \
             FROM information_schema.columns \
             WHERE lower(table_name) = lower(?) AND table_schema = COALESCE(?, DATABASE()) \
             ORDER BY ordinal_position"
                .into(),
            vec![Bind::Text(name.to_owned()), schema_bind],
        ),
        Driver::Sqlite => match schema {
            Some(s) => (
                "SELECT ?, name, type, \"notnull\" = 0, dflt_value, pk > 0 FROM pragma_table_info(?, ?)"
                    .into(),
                vec![
                    Bind::Text(s.to_owned()),
                    Bind::Text(name.to_owned()),
                    Bind::Text(s.to_owned()),
                ],
            ),
            None => (
                "SELECT 'main', name, type, \"notnull\" = 0, dflt_value, pk > 0 FROM pragma_table_info(?)"
                    .into(),
                vec![Bind::Text(name.to_owned())],
            ),
        },
    }
}

pub fn describe_table(
    conn: &DbConnection,
    table: &str,
    sql: String,
    binds: &[Bind],
    raw: RawResult,
) -> Result<SqlOutcome, SqlError> {
    let (_, name) = split(table);
    // Only schemas this connection may read count (a same-named table in an
    // unlisted schema is neither shown nor makes the name ambiguous).
    let visible: Vec<&Vec<Value>> = raw
        .rows
        .iter()
        .filter(|r| {
            let qualified = format!("{}.{name}", text(r.first()));
            !is_system_table(&qualified, conn.driver)
                && table_allowed(&qualified, &conn.table_allowlist, conn.driver)
        })
        .collect();
    let mut schemas: Vec<String> = visible.iter().map(|r| text(r.first())).collect();
    schemas.dedup();
    match schemas.len() {
        0 => {
            return Err(SqlError::Rejected(format!(
                "no table named `{table}` is visible; use list_tables to see what's there"
            )))
        }
        1 => {}
        _ => {
            return Err(SqlError::Rejected(format!(
                "`{table}` exists in several schemas ({}); pass it as schema.table",
                schemas.join(", ")
            )))
        }
    }
    let tables = vec![format!("{}.{name}", schemas[0]), name.to_owned()];
    let rows = visible
        .iter()
        .map(|r| {
            let column = text(r.get(1));
            let masked = conn
                .column_masks
                .iter()
                .any(|m| m.applies(&column, &tables));
            vec![
                Value::String(column),
                Value::String(text(r.get(2))),
                Value::Bool(truthy(r.get(3))),
                r.get(4).cloned().unwrap_or(Value::Null),
                Value::Bool(truthy(r.get(5))),
                Value::Bool(masked),
            ]
        })
        .collect();
    Ok(SqlOutcome {
        sql,
        params: binds.iter().map(Bind::to_json).collect(),
        columns: [
            "column",
            "type",
            "nullable",
            "default",
            "primary_key",
            "masked",
        ]
        .map(String::from)
        .to_vec(),
        rows,
        truncated: raw.truncated,
        masked_columns: Vec::new(),
        elapsed_ms: 0,
        write: None,
    })
}

#[cfg(test)]
#[path = "../tests/unit/explore.rs"]
mod tests;
