//! Database connections and SQL tools: curated read-only queries and the
//! opt-in exploration tools (`list_tables`, `describe_table`, `run_sql`).

use std::collections::HashSet;
use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::schema::{ArgBinding, ArgTarget, DerivedInput};
use crate::secret::SecretRef;
use crate::validate::{is_valid_name, ValidationErrors, MAX_NAME_LEN};

pub const DEFAULT_ROW_LIMIT: u32 = 100;
pub const MAX_ROW_LIMIT: u32 = 10_000;
pub const DEFAULT_SQL_TIMEOUT_MS: u32 = 10_000;
pub const MIN_SQL_TIMEOUT_MS: u32 = 100;
pub const MAX_SQL_TIMEOUT_MS: u32 = 300_000;
pub const DEFAULT_MAX_AFFECTED_ROWS: u32 = 100;
pub const MAX_SQL_LEN: usize = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Driver {
    Sqlite,
    Postgres,
    Mysql,
}

impl Driver {
    pub fn as_str(self) -> &'static str {
        match self {
            Driver::Sqlite => "sqlite",
            Driver::Postgres => "postgres",
            Driver::Mysql => "mysql",
        }
    }
}

/// What agents may do through a connection:
/// - `ReadOnly`: every tool only reads.
/// - `CuratedWrite`: saved queries may also write (INSERT/UPDATE/DELETE, and
///   DDL with `allow_ddl`); agent-written `run_sql` stays read-only.
/// - `FullAccess`: `run_sql` may write too.
///
/// Admin statements (GRANT, SET, COPY, …) are never allowed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessMode {
    #[default]
    ReadOnly,
    CuratedWrite,
    FullAccess,
}

/// A column whose values are replaced in results: `users.password_hash`,
/// `public.users.email` or `*.email` (any table). Matching ignores case.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ColumnMask {
    /// Table name (optionally schema-qualified), or `*`.
    pub table: String,
    pub column: String,
}

impl ColumnMask {
    pub fn parse(s: &str) -> Result<Self, String> {
        let ok = |p: &str| {
            !p.is_empty()
                && p.len() <= 128
                && p.chars()
                    .all(|c| c.is_alphanumeric() || matches!(c, '_' | '$' | '.'))
        };
        let (table, column) = s
            .rsplit_once('.')
            .ok_or("must look like `table.column` or `*.column`")?;
        if column.is_empty() || column.contains('.') || !ok(column) {
            return Err("column must be a plain column name".into());
        }
        if table != "*" && !ok(table) {
            return Err("table must be a table name (optionally `schema.table`) or `*`".into());
        }
        Ok(Self {
            table: table.to_owned(),
            column: column.to_owned(),
        })
    }

    /// Whether this mask covers `column` of a query that reads `tables`
    /// (names as written in the query, possibly schema-qualified).
    pub fn applies(&self, column: &str, tables: &[String]) -> bool {
        if !self.column.eq_ignore_ascii_case(column) {
            return false;
        }
        if self.table == "*" {
            return true;
        }
        let want = self.table.to_lowercase();
        tables.iter().any(|t| {
            let t = t.to_lowercase();
            // `users` matches `public.users` and vice versa.
            t == want
                || t.rsplit('.').next() == Some(want.as_str())
                || want.rsplit('.').next() == Some(t.as_str())
        })
    }
}

impl TryFrom<String> for ColumnMask {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        Self::parse(&s)
    }
}

impl From<ColumnMask> for String {
    fn from(m: ColumnMask) -> Self {
        m.to_string()
    }
}

impl fmt::Display for ColumnMask {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.table, self.column)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DbConnection {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub driver: Driver,
    /// Reference to the connection string (`env:`/`keyring:`), never the DSN itself.
    pub dsn_ref: SecretRef,
    pub access_mode: AccessMode,
    pub row_limit: u32,
    pub timeout_ms: u32,
    pub max_affected_rows: u32,
    pub allow_ddl: bool,
    pub table_allowlist: Vec<String>,
    pub column_masks: Vec<ColumnMask>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Create/replace payload for a connection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DbConnectionInput {
    pub name: String,
    pub driver: Driver,
    pub dsn_ref: SecretRef,
    #[serde(default)]
    pub access_mode: AccessMode,
    #[serde(default = "default_row_limit")]
    pub row_limit: u32,
    #[serde(default = "default_sql_timeout")]
    pub timeout_ms: u32,
    #[serde(default = "default_max_affected")]
    pub max_affected_rows: u32,
    #[serde(default)]
    pub allow_ddl: bool,
    #[serde(default)]
    pub table_allowlist: Vec<String>,
    #[serde(default)]
    pub column_masks: Vec<ColumnMask>,
}

fn default_row_limit() -> u32 {
    DEFAULT_ROW_LIMIT
}
fn default_sql_timeout() -> u32 {
    DEFAULT_SQL_TIMEOUT_MS
}
fn default_max_affected() -> u32 {
    DEFAULT_MAX_AFFECTED_ROWS
}

impl DbConnection {
    pub fn input(&self) -> DbConnectionInput {
        DbConnectionInput {
            name: self.name.clone(),
            driver: self.driver,
            dsn_ref: self.dsn_ref.clone(),
            access_mode: self.access_mode,
            row_limit: self.row_limit,
            timeout_ms: self.timeout_ms,
            max_affected_rows: self.max_affected_rows,
            allow_ddl: self.allow_ddl,
            table_allowlist: self.table_allowlist.clone(),
            column_masks: self.column_masks.clone(),
        }
    }
}

impl DbConnectionInput {
    pub fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errs = ValidationErrors::default();
        if !is_valid_name(&self.name) {
            errs.push(
                "name",
                format!("must be 1–{MAX_NAME_LEN} characters of a-z, A-Z, 0-9, _ or -"),
            );
        }
        if self.allow_ddl && self.access_mode == AccessMode::ReadOnly {
            errs.push("allow_ddl", "DDL needs curated_write or full_access access");
        }
        if !(1..=MAX_ROW_LIMIT).contains(&self.row_limit) {
            errs.push(
                "row_limit",
                format!("must be between 1 and {MAX_ROW_LIMIT}"),
            );
        }
        if !(MIN_SQL_TIMEOUT_MS..=MAX_SQL_TIMEOUT_MS).contains(&self.timeout_ms) {
            errs.push(
                "timeout_ms",
                format!("must be between {MIN_SQL_TIMEOUT_MS} and {MAX_SQL_TIMEOUT_MS}"),
            );
        }
        if !(1..=1_000_000).contains(&self.max_affected_rows) {
            errs.push("max_affected_rows", "must be between 1 and 1000000");
        }
        if self.table_allowlist.len() > 500 {
            errs.push("table_allowlist", "at most 500 entries");
        }
        for (i, t) in self.table_allowlist.iter().enumerate() {
            let ok = !t.is_empty()
                && t.len() <= 128
                && t.chars()
                    .all(|c| c.is_alphanumeric() || matches!(c, '_' | '$' | '.' | '*'));
            if !ok {
                errs.push(
                    format!("table_allowlist[{i}]"),
                    "must be a table name (optionally `schema.table`)",
                );
            }
        }
        if self.column_masks.len() > 200 {
            errs.push("column_masks", "at most 200 entries");
        }
        errs.into_result()
    }
}

/// A tool that runs SQL against a [`DbConnection`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SqlSpec {
    pub connection_id: Uuid,
    pub mode: SqlMode,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SqlMode {
    /// A fixed, read-only statement. `:name` placeholders are bound to
    /// `params`, never interpolated.
    Query {
        sql: String,
        #[serde(default)]
        params: Vec<SqlParam>,
    },
    /// Lists tables and views the connection may read (allowlist applies).
    ListTables,
    /// Columns, types, nullability and primary key of one allowed table.
    DescribeTable,
    /// Runs one agent-written read-only query against allowed tables.
    RunSql,
}

impl SqlMode {
    /// Snake-case name, as in the serialized `type` tag.
    pub fn as_str(&self) -> &'static str {
        match self {
            SqlMode::Query { .. } => "query",
            SqlMode::ListTables => "list_tables",
            SqlMode::DescribeTable => "describe_table",
            SqlMode::RunSql => "run_sql",
        }
    }
}

/// Argument names of the exploration tools.
pub const PATTERN_ARG: &str = "pattern";
pub const TABLE_ARG: &str = "table";
pub const SQL_ARG: &str = "sql";

/// Scalar JSON types a SQL param can have; values are bound with the matching
/// database type.
pub const SQL_PARAM_TYPES: [&str; 4] = ["string", "integer", "number", "boolean"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SqlParam {
    /// Placeholder name: `:name` in the SQL.
    pub name: String,
    /// JSON Schema; `type` must be one of [`SQL_PARAM_TYPES`].
    #[serde(default = "default_sql_param_schema")]
    pub schema: Value,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub description: Option<String>,
}

impl SqlParam {
    /// The declared scalar type (`string` when missing).
    pub fn json_type(&self) -> &str {
        self.schema
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("string")
    }
}

fn default_sql_param_schema() -> Value {
    json!({ "type": "string" })
}

/// `[A-Za-z_][A-Za-z0-9_]{0,63}`: usable both as a `:placeholder` and as an
/// MCP argument name.
pub fn is_valid_sql_param_name(s: &str) -> bool {
    let mut b = s.bytes();
    s.len() <= MAX_NAME_LEN
        && b.next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
        && b.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

impl SqlSpec {
    pub fn params(&self) -> &[SqlParam] {
        match &self.mode {
            SqlMode::Query { params, .. } => params,
            _ => &[],
        }
    }

    pub fn derive_input(&self) -> DerivedInput {
        let fixed = |name: &str, desc: &str, required: bool| {
            let Value::Object(schema) = json!({
                "type": "object",
                "properties": { name: { "type": "string", "description": desc } },
                "required": if required { vec![name] } else { vec![] },
                "additionalProperties": false,
            }) else {
                unreachable!("json! object literal")
            };
            let bindings = vec![ArgBinding {
                arg: name.to_owned(),
                target: ArgTarget::SqlParam {
                    name: name.to_owned(),
                },
                required,
            }];
            DerivedInput { schema, bindings }
        };
        match &self.mode {
            SqlMode::Query { .. } => {}
            SqlMode::ListTables => {
                return fixed(
                    PATTERN_ARG,
                    "Only tables whose name contains this text (case-insensitive).",
                    false,
                )
            }
            SqlMode::DescribeTable => {
                return fixed(
                    TABLE_ARG,
                    "Table name, optionally schema-qualified (`orders` or `sales.orders`).",
                    true,
                )
            }
            SqlMode::RunSql => {
                return fixed(
                    SQL_ARG,
                    "One read-only SELECT statement. Write values inline; no placeholders.",
                    true,
                )
            }
        }
        let mut properties = Map::new();
        let mut required = Vec::new();
        let mut bindings = Vec::new();
        for p in self.params() {
            let mut schema = p.schema.clone();
            if let (Some(desc), Some(obj)) = (&p.description, schema.as_object_mut()) {
                obj.entry("description")
                    .or_insert_with(|| Value::String(desc.clone()));
            }
            properties.insert(p.name.clone(), schema);
            if p.required {
                required.push(Value::String(p.name.clone()));
            }
            bindings.push(ArgBinding {
                arg: p.name.clone(),
                target: ArgTarget::SqlParam {
                    name: p.name.clone(),
                },
                required: p.required,
            });
        }
        let Value::Object(schema) = json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        }) else {
            unreachable!("json! object literal")
        };
        DerivedInput { schema, bindings }
    }

    /// Structural checks. Placeholder matching and the read-only SQL check
    /// need a SQL parser and run in the SQL executor.
    pub(crate) fn validate_into(&self, errs: &mut ValidationErrors) {
        let SqlMode::Query { sql, params } = &self.mode else {
            return;
        };
        if sql.trim().is_empty() {
            errs.push("kind.mode.sql", "must not be empty");
        } else if sql.len() > MAX_SQL_LEN {
            errs.push(
                "kind.mode.sql",
                format!("must be at most {MAX_SQL_LEN} bytes"),
            );
        }
        let mut seen = HashSet::new();
        for (i, p) in params.iter().enumerate() {
            let at = |f: &str| format!("kind.mode.params[{i}].{f}");
            if !is_valid_sql_param_name(&p.name) {
                errs.push(
                    at("name"),
                    "must start with a letter or _ and contain only a-z, A-Z, 0-9 or _ (max 64)",
                );
            }
            if !seen.insert(p.name.to_lowercase()) {
                errs.push(at("name"), format!("duplicate param `{}`", p.name));
            }
            match p.schema.get("type").and_then(Value::as_str) {
                _ if !p.schema.is_object() => {
                    errs.push(at("schema"), "must be a JSON Schema object")
                }
                Some(t) if SQL_PARAM_TYPES.contains(&t) => {}
                None => {}
                Some(t) => errs.push(
                    at("schema"),
                    format!("type `{t}` is not supported; use string, integer, number or boolean"),
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn(v: Value) -> DbConnectionInput {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn connection_defaults_and_rules() {
        let c = conn(json!({ "name": "main", "driver": "postgres", "dsn_ref": "env:DB" }));
        c.validate().unwrap();
        assert_eq!(c.access_mode, AccessMode::ReadOnly);
        assert_eq!(c.row_limit, DEFAULT_ROW_LIMIT);
        assert_eq!(c.timeout_ms, DEFAULT_SQL_TIMEOUT_MS);

        let c = conn(json!({
            "name": "bad name", "driver": "mysql", "dsn_ref": "env:DB",
            "allow_ddl": true, "row_limit": 0, "timeout_ms": 5,
            "table_allowlist": ["ok", "no spaces"]
        }));
        let e = c.validate().unwrap_err();
        for f in [
            "name",
            "allow_ddl",
            "row_limit",
            "timeout_ms",
            "table_allowlist[1]",
        ] {
            assert!(e.has(f), "{f}: {e}");
        }
        // Raw DSNs never deserialize.
        assert!(serde_json::from_value::<DbConnectionInput>(json!({
            "name": "x", "driver": "postgres", "dsn_ref": "postgres://u:pw@h/db"
        }))
        .is_err());
    }

    #[test]
    fn column_masks() {
        let m = ColumnMask::parse("users.password_hash").unwrap();
        assert_eq!(m.to_string(), "users.password_hash");
        assert!(m.applies("PASSWORD_HASH", &["public.users".into()]));
        assert!(!m.applies("password_hash", &["orders".into()]));
        let any = ColumnMask::parse("*.email").unwrap();
        assert!(any.applies("email", &[]));
        let q = ColumnMask::parse("public.users.email").unwrap();
        assert_eq!(q.table, "public.users");
        assert!(q.applies("email", &["users".into()]));
        for bad in ["email", "users.", ".email", "a b.c", "t.c;"] {
            assert!(ColumnMask::parse(bad).is_err(), "{bad}");
        }
        assert_eq!(
            serde_json::to_value(&m).unwrap(),
            json!("users.password_hash")
        );
    }

    #[test]
    fn sql_spec_schema_and_validation() {
        let s: SqlSpec = serde_json::from_value(json!({
            "connection_id": Uuid::nil(),
            "mode": { "type": "query", "sql": "SELECT * FROM users WHERE id = :id AND name LIKE :q",
                "params": [
                    { "name": "id", "schema": { "type": "integer" }, "required": true, "description": "User id" },
                    { "name": "q" }
                ] }
        }))
        .unwrap();
        let d = s.derive_input();
        assert_eq!(
            Value::Object(d.schema),
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "integer", "description": "User id" },
                    "q": { "type": "string" }
                },
                "required": ["id"],
                "additionalProperties": false
            })
        );
        let mut errs = ValidationErrors::default();
        s.validate_into(&mut errs);
        assert!(errs.is_empty(), "{errs}");

        let bad: SqlSpec = serde_json::from_value(json!({
            "connection_id": Uuid::nil(),
            "mode": { "type": "query", "sql": " ", "params": [
                { "name": "1x" }, { "name": "a", "schema": { "type": "array" } }, { "name": "A" }
            ] }
        }))
        .unwrap();
        let mut errs = ValidationErrors::default();
        bad.validate_into(&mut errs);
        for f in [
            "kind.mode.sql",
            "kind.mode.params[0].name",
            "kind.mode.params[1].schema",
            "kind.mode.params[2].name",
        ] {
            assert!(errs.has(f), "{f}: {errs}");
        }
    }

    #[test]
    fn exploration_modes_have_fixed_inputs() {
        let spec = |mode: Value| -> SqlSpec {
            serde_json::from_value(json!({ "connection_id": Uuid::nil(), "mode": mode })).unwrap()
        };
        let d = spec(json!({ "type": "run_sql" })).derive_input();
        assert_eq!(d.schema["required"], json!(["sql"]));
        assert_eq!(d.bindings[0].arg, "sql");
        let d = spec(json!({ "type": "list_tables" })).derive_input();
        assert_eq!(d.schema["required"], json!([]));
        assert!(d.schema["properties"]["pattern"].is_object());
        let s = spec(json!({ "type": "describe_table" }));
        assert_eq!(s.mode.as_str(), "describe_table");
        assert!(s.params().is_empty());
        let mut errs = ValidationErrors::default();
        s.validate_into(&mut errs);
        assert!(errs.is_empty());
    }

    #[test]
    fn write_modes_and_ddl() {
        for mode in ["curated_write", "full_access"] {
            conn(json!({ "name": "w", "driver": "sqlite", "dsn_ref": "env:DB", "access_mode": mode, "allow_ddl": true }))
                .validate()
                .unwrap();
        }
    }
}
