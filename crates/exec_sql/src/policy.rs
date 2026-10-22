//! Access-mode gating, save-time and agent-SQL checks, binds and result masking.

use forge_core::{
    AccessMode, ArgBinding, ArgTarget, ColumnMask, DbConnection, DbConnectionInput, DerivedInput,
    SqlMode, SqlSpec, ValidationErrors,
};
use serde_json::{json, Map, Value};

use crate::{
    error::SqlError, guard, outcome::SqlOutcome, outcome::DRY_RUN_ARG, Bind, RawResult,
    StatementClass, MASKED,
};

/// Whether the access mode lets this statement class run. Agent-written SQL
/// (`run_sql`) may only write under `FullAccess`; saved queries may under
/// `CuratedWrite` too. DDL also needs `allow_ddl`; admin statements never run.
pub fn gate(
    q: &guard::CheckedQuery,
    mode: AccessMode,
    allow_ddl: bool,
    agent_sql: bool,
) -> Result<(), String> {
    let reason = || {
        q.reason
            .clone()
            .unwrap_or_else(|| "only SELECT queries are allowed".into())
    };
    match q.class {
        StatementClass::Read => Ok(()),
        StatementClass::Admin => Err(format!(
            "{}. Admin statements (GRANT, SET, COPY, PRAGMA, CALL, …) are never allowed",
            reason()
        )),
        StatementClass::Write | StatementClass::Ddl => {
            match (mode, agent_sql) {
                (AccessMode::ReadOnly, _) => {
                    return Err(format!("{} (this connection is read-only)", reason()))
                }
                (AccessMode::CuratedWrite, true) => {
                    return Err(format!(
                        "{} (this connection only allows writes through saved tools)",
                        reason()
                    ))
                }
                _ => {}
            }
            if q.class == StatementClass::Ddl && !allow_ddl {
                return Err(
                    "schema changes (CREATE, ALTER, DROP, TRUNCATE) are turned off for this connection"
                        .into(),
                );
            }
            match &q.missing_where {
                Some(m) => Err(m.clone()),
                None => Ok(()),
            }
        }
    }
}

/// Whether a tool can change data on this connection (and so gets
/// `dry_run`, a destructive annotation and an audit trail).
pub fn tool_writes(spec: &SqlSpec, conn: &DbConnection) -> bool {
    match &spec.mode {
        SqlMode::Query { sql, .. } => guard::bind_placeholders(sql, conn.driver)
            .and_then(|b| guard::check_statement(&b.sql, conn.driver))
            .is_ok_and(|q| matches!(q.class, StatementClass::Write | StatementClass::Ddl)),
        SqlMode::RunSql => conn.access_mode == AccessMode::FullAccess,
        SqlMode::ListTables | SqlMode::DescribeTable => false,
    }
}

/// The tool's MCP input: its own arguments, plus `dry_run` when it writes.
pub fn tool_input(spec: &SqlSpec, conn: &DbConnection) -> DerivedInput {
    let mut d = spec.derive_input();
    if tool_writes(spec, conn) {
        if let Some(props) = d
            .schema
            .get_mut("properties")
            .and_then(Value::as_object_mut)
        {
            props.insert(
                DRY_RUN_ARG.into(),
                json!({
                    "type": "boolean",
                    "description": "If true, run the statement, report how many rows it would change, then undo it."
                }),
            );
        }
        d.bindings.push(ArgBinding {
            arg: DRY_RUN_ARG.into(),
            target: ArgTarget::SqlParam {
                name: DRY_RUN_ARG.into(),
            },
            required: false,
        });
    }
    d
}

/// Save-time checks for a query tool: every `:name` has a param and vice
/// versa, and the statement passes the guard and the connection's access mode.
pub fn check_query_tool(spec: &SqlSpec, conn: &DbConnectionInput) -> Result<(), ValidationErrors> {
    let driver = conn.driver;
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
    match guard::check_statement(&bound.sql, driver) {
        Err(e) => errs.push("kind.mode.sql", e),
        Ok(q) => {
            if let Err(e) = gate(&q, conn.access_mode, conn.allow_ddl, false) {
                errs.push("kind.mode.sql", e);
            }
        }
    }
    errs.into_result()
}

/// Turns validated arguments into positional binds.
pub(crate) fn binds_for(
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
pub(crate) fn finish(
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
        write: None,
    }
}

pub(crate) fn allowed_list(conn: &DbConnection) -> String {
    if conn.table_allowlist.is_empty() {
        "any non-system table".into()
    } else {
        conn.table_allowlist.join(", ")
    }
}

/// System catalogs are never available; others need the allowlist.
pub(crate) fn check_table_access(conn: &DbConnection, table: &str) -> Result<(), SqlError> {
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

/// The checks agent-written SQL gets on top of the guard: allowed by the
/// access mode, no placeholders, only allowed tables, no way around masks.
pub fn check_agent_sql(conn: &DbConnection, sql: &str) -> Result<guard::CheckedQuery, SqlError> {
    match guard::bind_placeholders(sql, conn.driver) {
        Ok(b) if b.names.is_empty() => {}
        _ => {
            return Err(SqlError::Rejected(
                "placeholders aren't supported here; write values inline".into(),
            ))
        }
    }
    let checked = guard::check_statement(sql, conn.driver).map_err(SqlError::Rejected)?;
    gate(&checked, conn.access_mode, conn.allow_ddl, true).map_err(SqlError::Rejected)?;
    for t in &checked.tables {
        check_table_access(conn, t)?;
    }
    guard::check_masks(&checked, &conn.column_masks).map_err(SqlError::Rejected)?;
    Ok(checked)
}

/// Replaces masked columns' values; returns the masked column names.
pub(crate) fn apply_masks(
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
