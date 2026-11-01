//! MCP resources and prompts. Resources are the project's text documents
//! (`forge://resources/{name}`), database schemas (`schema://{connection}`,
//! only for connections whose describe tool is on, so schemas follow the
//! same opt-in as exploration) and, with `share_specs`, imported spec
//! documents (`spec://{id}`). Prompts are templates with arguments.

use std::fmt::Write as _;

use forge_core::{
    DbConnection, NosqlSpec, ProjectSettings, SqlMode, SqlSpec, ToolDef, ToolKind, DESCRIBE_TARGET,
    LIST_TARGETS, RESOURCE_URI_PREFIX,
};
use forge_store::{Store, StoreError};
use rmcp::model::{Prompt, PromptArgument, Resource};
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::Executors;

pub const SCHEMA_PREFIX: &str = "schema://";
pub const SPEC_PREFIX: &str = "spec://";

/// Most tables / collections described in one schema resource.
const MAX_DESCRIBED: usize = 50;

/// Connections that offer a schema: their describe tool exists and is on.
fn schema_connections<'a>(tools: &[ToolDef], conns: &'a [DbConnection]) -> Vec<&'a DbConnection> {
    conns
        .iter()
        .filter(|c| {
            tools.iter().any(|t| {
                t.enabled
                    && match &t.kind {
                        ToolKind::Sql(s) => {
                            s.connection_id == c.id && s.mode == SqlMode::DescribeTable
                        }
                        ToolKind::Nosql(s) => {
                            s.connection_id == c.id && s.operation == DESCRIBE_TARGET
                        }
                        _ => false,
                    }
            })
        })
        .collect()
}

fn spec_mime(raw: &str) -> &'static str {
    if raw.trim_start().starts_with('{') {
        "application/json"
    } else {
        "application/yaml"
    }
}

pub async fn list_resources(
    store: &Store,
    project_id: Uuid,
    settings: &ProjectSettings,
) -> Result<Vec<Resource>, StoreError> {
    let mut out: Vec<Resource> = store
        .list_resources(project_id)
        .await?
        .into_iter()
        .map(|r| {
            let mut res =
                Resource::new(r.uri(), r.name.clone()).with_mime_type(r.mime_type.clone());
            if !r.description.is_empty() {
                res = res.with_description(r.description.clone());
            }
            res
        })
        .collect();
    let tools = store.list_tools(project_id).await?;
    let conns = store.list_connections(project_id).await?;
    for c in schema_connections(&tools, &conns) {
        out.push(
            Resource::new(format!("{SCHEMA_PREFIX}{}", c.name), format!("{}_schema", c.name))
                .with_description(format!(
                    "Tables, collections or indexes of the `{}` database the agent may use, with their fields.",
                    c.name
                ))
                .with_mime_type("text/markdown"),
        );
    }
    if settings.share_specs {
        for s in store.list_specs(project_id).await? {
            out.push(
                Resource::new(
                    format!("{SPEC_PREFIX}{}", s.id),
                    format!("spec_{}", s.id.simple()),
                )
                .with_description(
                    format!("The imported API description: {} {}", s.title, s.version)
                        .trim_end()
                        .to_owned(),
                ),
            );
        }
    }
    Ok(out)
}

/// What a resource read returns, or why there's nothing.
pub enum Read {
    Found { text: String, mime: String },
    NotFound,
}

pub async fn read_resource(
    store: &Store,
    exec: &Executors,
    project_id: Uuid,
    settings: &ProjectSettings,
    uri: &str,
) -> Result<Read, StoreError> {
    if let Some(name) = uri.strip_prefix(RESOURCE_URI_PREFIX) {
        let found = store
            .list_resources(project_id)
            .await?
            .into_iter()
            .find(|r| r.name == name);
        return Ok(match found {
            Some(r) => Read::Found {
                text: r.content,
                mime: r.mime_type,
            },
            None => Read::NotFound,
        });
    }
    if let Some(name) = uri.strip_prefix(SCHEMA_PREFIX) {
        let tools = store.list_tools(project_id).await?;
        let conns = store.list_connections(project_id).await?;
        let Some(conn) = schema_connections(&tools, &conns)
            .into_iter()
            .find(|c| c.name == name)
            .cloned()
        else {
            return Ok(Read::NotFound);
        };
        let text = schema_markdown(exec, &conn).await;
        return Ok(Read::Found {
            text,
            mime: "text/markdown".into(),
        });
    }
    if let Some(id) = uri.strip_prefix(SPEC_PREFIX) {
        if !settings.share_specs {
            return Ok(Read::NotFound);
        }
        let Ok(id) = id.parse::<Uuid>() else {
            return Ok(Read::NotFound);
        };
        return Ok(match store.get_spec_raw(project_id, id).await {
            Ok(raw) => Read::Found {
                mime: spec_mime(&raw).into(),
                text: raw,
            },
            Err(StoreError::NotFound(_)) => Read::NotFound,
            Err(e) => return Err(e),
        });
    }
    Ok(Read::NotFound)
}

fn cell(v: &Value) -> String {
    let s = match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    s.replace('|', "\\|").replace('\n', " ")
}

/// A markdown table of exploration output.
fn table(columns: &[String], rows: &[Vec<Value>]) -> String {
    let mut t = format!(
        "| {} |\n|{}\n",
        columns.join(" | "),
        " --- |".repeat(columns.len())
    );
    for r in rows {
        let cells: Vec<String> = r.iter().map(cell).collect();
        let _ = writeln!(t, "| {} |", cells.join(" | "));
    }
    t
}

/// The schema of a connection, through its exploration queries (so the
/// allowlist, system-catalog rules and masks apply).
async fn schema_markdown(exec: &Executors, conn: &DbConnection) -> String {
    let mut md = format!(
        "# `{}` ({})\n\n",
        conn.name,
        crate::driver_name(conn.driver)
    );
    let args = |v: Value| v.as_object().cloned().unwrap_or_default();
    if conn.driver.is_sql() {
        let spec = |mode| SqlSpec {
            connection_id: conn.id,
            mode,
        };
        let tables = match exec
            .sql
            .run(conn, &spec(SqlMode::ListTables), &Map::new())
            .await
            .result
        {
            Ok(o) => o.rows,
            Err(e) => return format!("{md}Couldn't list tables: {e}\n"),
        };
        for t in tables.iter().take(MAX_DESCRIBED) {
            let name = format!("{}.{}", cell(&t[0]), cell(&t[1]));
            let _ = writeln!(md, "## {name}\n");
            match exec
                .sql
                .run(
                    conn,
                    &spec(SqlMode::DescribeTable),
                    &args(json!({ "table": name })),
                )
                .await
                .result
            {
                Ok(o) => md.push_str(&table(&o.columns, &o.rows)),
                Err(e) => {
                    let _ = writeln!(md, "Couldn't describe it: {e}");
                }
            }
            md.push('\n');
        }
        if tables.len() > MAX_DESCRIBED {
            let _ = writeln!(
                md,
                "… and {} more tables (use the list tool).",
                tables.len() - MAX_DESCRIBED
            );
        }
    } else {
        let spec = |op: &str| NosqlSpec {
            connection_id: conn.id,
            operation: op.into(),
            target: String::new(),
            body: Value::Null,
            params: vec![],
        };
        let names: Vec<String> = match exec
            .nosql
            .run(conn, &spec(LIST_TARGETS), &Map::new())
            .await
            .result
        {
            Ok(o) => o
                .documents
                .iter()
                .filter_map(|d| d["name"].as_str().map(str::to_owned))
                .collect(),
            Err(e) => return format!("{md}Couldn't list what's there: {e}\n"),
        };
        for n in names.iter().take(MAX_DESCRIBED) {
            let _ = writeln!(md, "## {n}\n");
            match exec
                .nosql
                .run(conn, &spec(DESCRIBE_TARGET), &args(json!({ "target": n })))
                .await
                .result
            {
                Ok(o) => {
                    let body = serde_json::to_string_pretty(
                        &o.documents.first().cloned().unwrap_or(Value::Null),
                    )
                    .unwrap_or_default();
                    let _ = writeln!(md, "```json\n{body}\n```");
                    if !o.masked_fields.is_empty() {
                        let _ = writeln!(md, "Masked fields: {}", o.masked_fields.join(", "));
                    }
                }
                Err(e) => {
                    let _ = writeln!(md, "Couldn't describe it: {e}");
                }
            }
            md.push('\n');
        }
    }
    md
}

pub async fn list_prompts(store: &Store, project_id: Uuid) -> Result<Vec<Prompt>, StoreError> {
    Ok(store
        .list_prompts(project_id)
        .await?
        .into_iter()
        .map(|p| {
            let args: Vec<PromptArgument> = p
                .arguments
                .iter()
                .map(|a| {
                    let mut x = PromptArgument::new(a.name.clone()).with_required(a.required);
                    if !a.description.is_empty() {
                        x = x.with_description(a.description.clone());
                    }
                    x
                })
                .collect();
            let desc = (!p.description.is_empty()).then(|| p.description.clone());
            Prompt::new(p.name.clone(), desc, Some(args))
        })
        .collect())
}
