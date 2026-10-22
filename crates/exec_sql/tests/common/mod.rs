//! Shared helpers for SQL executor tests.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Arc;

use forge_core::{DbConnection, SecretRef, SecretResolver, SqlSpec};
use forge_exec_sql::SqlExecutor;
use serde_json::{json, Map, Value};
use uuid::Uuid;

/// Resolves `env:NAME` from a fixed map.
pub struct Secrets(pub HashMap<String, String>);

impl SecretResolver for Secrets {
    fn resolve(&self, r: &SecretRef) -> Result<String, String> {
        self.0
            .get(r.name())
            .cloned()
            .ok_or_else(|| format!("secret `{r}` is not set"))
    }
}

/// An executor plus a saved-looking connection pointing at `dsn`.
pub fn setup(driver: &str, dsn: &str, extra: Value) -> (SqlExecutor, DbConnection) {
    let exec = SqlExecutor::new(Arc::new(Secrets(HashMap::from([(
        "DB".to_owned(),
        dsn.to_owned(),
    )]))));
    let mut v = json!({
        "id": Uuid::new_v4(), "project_id": Uuid::new_v4(), "name": "db",
        "driver": driver, "dsn_ref": "env:DB", "access_mode": "read_only",
        "row_limit": 100, "timeout_ms": 2000, "max_affected_rows": 100, "allow_ddl": false,
        "table_allowlist": [], "column_masks": [],
        "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z"
    });
    for (k, val) in extra.as_object().unwrap() {
        v[k] = val.clone();
    }
    (exec, serde_json::from_value(v).unwrap())
}

/// A curated query spec.
pub fn query(sql: &str, params: Value) -> SqlSpec {
    serde_json::from_value(json!({
        "connection_id": Uuid::nil(),
        "mode": { "type": "query", "sql": sql, "params": params }
    }))
    .unwrap()
}

pub fn args(v: Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}

/// An exploration tool spec (`list_tables`, `describe_table`, `run_sql`).
pub fn mode(kind: &str) -> SqlSpec {
    serde_json::from_value(json!({ "connection_id": Uuid::nil(), "mode": { "type": kind } }))
        .unwrap()
}
