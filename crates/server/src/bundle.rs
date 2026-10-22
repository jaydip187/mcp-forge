//! `tools.yaml`: a project's auth configs, DB connections and tools as one
//! portable file.
//!
//! Auth configs and connections carry only secret *references*
//! (`env:`/`keyring:`); a raw value fails to parse, both on export and on
//! import. Tools point at them by name (`auth: github`, `connection: shop`),
//! not by id, so files move between machines. Imported tools become manual
//! tools.

use std::collections::HashMap;

use forge_core::{AuthConfigInput, DbConnectionInput, ToolInput, ToolKind, ValidationErrors};
use forge_store::{BundleTool, Store, StoreError};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

pub const BUNDLE_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleProject {
    pub name: String,
    pub slug: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct BundleFile {
    version: u32,
    #[serde(default)]
    project: Option<BundleProject>,
    #[serde(default)]
    auth_configs: Vec<AuthConfigInput>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    db_connections: Vec<DbConnectionInput>,
    #[serde(default)]
    tools: Vec<Value>,
}

/// A parsed, validated-for-shape bundle.
#[derive(Debug)]
pub struct Bundle {
    pub project: Option<BundleProject>,
    pub auth_configs: Vec<AuthConfigInput>,
    pub db_connections: Vec<DbConnectionInput>,
    /// Each tool with the names of its auth config / connection.
    pub tools: Vec<BundleTool>,
}

/// Serializes a project to YAML.
pub async fn export_project(store: &Store, project_id: Uuid) -> Result<String, StoreError> {
    let project = store.get_project(project_id).await?;
    let auths = store.list_auth_configs(project_id).await?;
    let names: HashMap<Uuid, String> = auths.iter().map(|a| (a.id, a.name.clone())).collect();
    let conns = store.list_connections(project_id).await?;
    let conn_names: HashMap<Uuid, String> = conns.iter().map(|c| (c.id, c.name.clone())).collect();
    let tools = store
        .list_tools(project_id)
        .await?
        .into_iter()
        .map(|t| {
            let auth = t.kind.auth_ref().and_then(|id| names.get(&id).cloned());
            let conn = t
                .kind
                .connection_ref()
                .and_then(|id| conn_names.get(&id).cloned());
            let mut v = serde_json::to_value(t.input()).expect("IR serializes");
            tidy(&mut v);
            let obj = v.as_object_mut().expect("tool is an object");
            if let Some(a) = auth {
                obj.insert("auth".into(), Value::String(a));
            }
            if let Some(c) = conn {
                obj.insert("connection".into(), Value::String(c));
            }
            // Keep `kind` last so the short fields read first.
            let kind = obj.remove("kind");
            if let Some(k) = kind {
                obj.insert("kind".into(), k);
            }
            v
        })
        .collect();
    let file = BundleFile {
        version: BUNDLE_VERSION,
        project: Some(BundleProject {
            name: project.name,
            slug: project.slug,
        }),
        auth_configs: auths
            .into_iter()
            .map(|a| AuthConfigInput {
                name: a.name,
                kind: a.kind,
                secret_ref: a.secret_ref,
            })
            .collect(),
        db_connections: conns.iter().map(|c| c.input()).collect(),
        tools,
    };
    let yaml = serde_yaml_ng::to_string(&file).expect("bundle serializes");
    Ok(format!(
        "# mcp-forge tools export. Secrets are references (env:NAME / keyring:NAME), never values.\n{yaml}"
    ))
}

/// Drops null/empty noise from an exported tool so the YAML stays readable.
fn tidy(tool: &mut Value) {
    let drop_null = |m: &mut Map<String, Value>, keys: &[&str]| {
        for k in keys {
            if m.get(*k).is_some_and(Value::is_null) {
                m.remove(*k);
            }
        }
    };
    let Some(obj) = tool.as_object_mut() else {
        return;
    };
    drop_null(obj, &["response_shape"]);
    if obj.get("description").and_then(Value::as_str) == Some("") {
        obj.remove("description");
    }
    if obj
        .get("tags")
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty)
    {
        obj.remove("tags");
    }
    if let Some(kind) = obj.get_mut("kind").and_then(Value::as_object_mut) {
        kind.remove("auth_ref");
        kind.remove("connection_id");
        drop_null(kind, &["body_schema"]);
        if let Some(params) = kind
            .get_mut("mode")
            .and_then(|m| m.get_mut("params"))
            .and_then(Value::as_array_mut)
        {
            for p in params.iter_mut().filter_map(Value::as_object_mut) {
                drop_null(p, &["description"]);
                if p.get("required") == Some(&Value::Bool(false)) {
                    p.remove("required");
                }
            }
        }
        if kind
            .get("params")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
        {
            kind.remove("params");
        }
        if let Some(params) = kind.get_mut("params").and_then(Value::as_array_mut) {
            for p in params.iter_mut().filter_map(Value::as_object_mut) {
                drop_null(p, &["description", "fixed_value"]);
                if p.get("required") == Some(&Value::Bool(false)) {
                    p.remove("required");
                }
            }
        }
    }
}

/// Parses YAML (or JSON) into a bundle. Errors name the offending entry.
pub fn parse_bundle(text: &str) -> Result<Bundle, ValidationErrors> {
    let file: BundleFile = serde_yaml_ng::from_str(text)
        .map_err(|e| ValidationErrors::single("content", format!("not a valid tools file: {e}")))?;
    if file.version != BUNDLE_VERSION {
        return Err(ValidationErrors::single(
            "version",
            format!(
                "unsupported version {}; this mcp-forge reads version {BUNDLE_VERSION}",
                file.version
            ),
        ));
    }
    let mut errs = ValidationErrors::default();
    let mut tools = Vec::with_capacity(file.tools.len());
    for (i, mut raw) in file.tools.into_iter().enumerate() {
        let auth = match raw.as_object_mut().and_then(|o| o.remove("auth")) {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s),
            Some(_) => {
                errs.push(
                    format!("tools[{i}].auth"),
                    "must be the name of an auth config",
                );
                None
            }
        };
        let connection = match raw.as_object_mut().and_then(|o| o.remove("connection")) {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s),
            Some(_) => {
                errs.push(
                    format!("tools[{i}].connection"),
                    "must be the name of a DB connection",
                );
                None
            }
        };
        if raw.pointer("/kind/auth_ref").is_some_and(|v| !v.is_null()) {
            errs.push(
                format!("tools[{i}].kind.auth_ref"),
                "use `auth: <auth config name>` instead of an id",
            );
            continue;
        }
        if raw
            .pointer("/kind/connection_id")
            .is_some_and(|v| !v.is_null())
        {
            errs.push(
                format!("tools[{i}].kind.connection_id"),
                "use `connection: <connection name>` instead of an id",
            );
            continue;
        }
        // Resolved from `connection` by the store.
        if let Some(kind) = raw.get_mut("kind").and_then(Value::as_object_mut) {
            if kind.get("type").and_then(Value::as_str) == Some("sql") {
                kind.insert(
                    "connection_id".into(),
                    Value::String(Uuid::nil().to_string()),
                );
            }
        }
        match serde_json::from_value::<ToolInput>(raw) {
            Ok(input) => tools.push(BundleTool {
                input,
                auth,
                connection,
            }),
            Err(e) => errs.push(format!("tools[{i}]"), e.to_string()),
        }
    }
    errs.into_result()?;
    Ok(Bundle {
        project: file.project,
        auth_configs: file.auth_configs,
        db_connections: file.db_connections,
        tools,
    })
}

/// Connections with `full_access` only import with explicit confirmation.
pub fn check_full_access(bundle: &Bundle, confirmed: bool) -> Result<(), StoreError> {
    if confirmed {
        return Ok(());
    }
    let mut errs = ValidationErrors::default();
    for (i, c) in bundle.db_connections.iter().enumerate() {
        if c.access_mode == forge_core::AccessMode::FullAccess {
            errs.push(
                format!("db_connections[{i}].access_mode"),
                "full_access lets agents change and delete data; confirm the import to allow it",
            );
        }
    }
    errs.into_result().map_err(StoreError::from)
}

/// Checks each SQL tool's statement against its connection's dialect (from
/// the bundle, else already in the project).
pub async fn check_sql_tools(
    store: &Store,
    project_id: Uuid,
    bundle: &Bundle,
) -> Result<(), StoreError> {
    let mut drivers: HashMap<String, DbConnectionInput> = store
        .list_connections(project_id)
        .await?
        .into_iter()
        .map(|c| (c.name.clone(), c.input()))
        .collect();
    for c in &bundle.db_connections {
        drivers.insert(c.name.clone(), c.clone());
    }
    let mut errs = ValidationErrors::default();
    for (i, t) in bundle.tools.iter().enumerate() {
        let ToolKind::Sql(spec) = &t.input.kind else {
            continue;
        };
        // Unknown connection names are reported by the store import.
        let Some(conn) = t.connection.as_ref().and_then(|n| drivers.get(n)) else {
            continue;
        };
        if let Err(e) = forge_exec_sql::check_query_tool(spec, conn) {
            for f in e.errors {
                errs.push(format!("tools[{i}].{}", f.field), f.message);
            }
        }
    }
    errs.into_result().map_err(StoreError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_raw_secrets_and_bad_versions() {
        let raw = "version: 1\nauth_configs:\n- name: gh\n  kind: {type: bearer}\n  secret_ref: ghp_supersecret\n";
        let e = parse_bundle(raw).unwrap_err();
        assert!(e.to_string().contains("not a raw secret"), "{e}");
        assert!(parse_bundle("version: 9\n").unwrap_err().has("version"));
        assert!(parse_bundle("not: [valid").unwrap_err().has("content"));
    }

    #[test]
    fn tool_auth_by_name() {
        let b = parse_bundle(
            "version: 1\ntools:\n- name: t\n  auth: gh\n  kind: {type: http, method: GET, base_url: 'https://x.io', path_template: /}\n",
        )
        .unwrap();
        assert_eq!(b.tools[0].auth.as_deref(), Some("gh"));
        assert!(b.tools[0].input.enabled, "defaults apply");
        let e = parse_bundle(
            "version: 1\ntools:\n- name: t\n  kind: {type: http, method: GET, base_url: 'https://x.io', path_template: /, auth_ref: 00000000-0000-0000-0000-000000000000}\n",
        )
        .unwrap_err();
        assert!(e.has("tools[0].kind.auth_ref"));
    }
}
