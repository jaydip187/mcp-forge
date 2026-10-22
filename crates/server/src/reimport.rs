//! Re-import a stored spec: refresh tools from a new version of the spec,
//! keep the user's edits (overrides), and report what changed.
//!
//! Matching: a tool belongs to the operation with the same `op_id`
//! (operationId, or `"METHOD /path"` when the spec has none).

use std::collections::{HashMap, HashSet};

use axum::extract::{DefaultBodyLimit, State};
use axum::routing::post;
use axum::{Json, Router};
use forge_core::{changed_fields, merge_overrides, ToolDef, ToolInput, ToolKind, ValidationErrors};
use forge_importers::openapi3::{BuiltOperation, Imported};
use forge_importers::unique_name;
use forge_store::{NewSpec, SpecRecord};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::api::{ApiError, ApiJson, ApiPath};
use crate::import::{fetch_spec, run_import, MAX_SPEC_BYTES};
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/projects/{project_id}/specs/{spec_id}/reimport/preview",
            post(preview).layer(DefaultBodyLimit::max(MAX_SPEC_BYTES)),
        )
        .route(
            "/projects/{project_id}/specs/{spec_id}/reimport",
            post(commit).layer(DefaultBodyLimit::max(MAX_SPEC_BYTES)),
        )
}

/// One existing tool whose spec values changed.
struct Change<'a> {
    current: ToolDef,
    merged: ToolInput,
    fields: Vec<&'static str>,
    op: &'a BuiltOperation,
}

#[derive(Default)]
struct Plan<'a> {
    changed: Vec<Change<'a>>,
    unchanged: Vec<ToolDef>,
    /// The operation still exists but can no longer be imported; tool left as is.
    blocked: Vec<(ToolDef, &'a BuiltOperation)>,
    /// New operations with their final (unique) names.
    added: Vec<(&'a BuiltOperation, String)>,
    /// Tools whose operation is gone from the spec.
    removed: Vec<ToolDef>,
}

/// Works out the re-import without touching the store.
fn plan<'a>(
    imported: &'a Imported,
    from_spec: Vec<ToolDef>,
    other_names: HashSet<String>,
    auth_ref: Option<Uuid>,
) -> Plan<'a> {
    let any_auth = imported.operations.iter().any(|o| o.needs_auth);
    let by_op_id: HashMap<String, &BuiltOperation> =
        imported.operations.iter().map(|o| (o.op_id(), o)).collect();
    let mut taken = other_names;
    let mut matched_keys = HashSet::new();
    let mut out = Plan::default();

    let mut from_spec = from_spec;
    from_spec.sort_by(|a, b| a.name.cmp(&b.name));
    for tool in from_spec {
        let op = tool.source.op_id().and_then(|id| by_op_id.get(id).copied());
        let Some(op) = op else {
            out.removed.push(tool);
            continue;
        };
        matched_keys.insert(op.key.as_str());
        if !op.supported {
            taken.insert(tool.name.clone());
            out.blocked.push((tool, op));
            continue;
        }
        let mut base = op.tool.clone();
        if op.needs_auth || !any_auth {
            let ToolKind::Http(h) = &mut base.kind;
            h.auth_ref = auth_ref;
        }
        let mut merged = merge_overrides(&tool.input(), &tool.overrides, &base);
        if merged.name != tool.name || taken.contains(&merged.name) {
            merged.name = unique_name(merged.name, &mut taken);
        } else {
            taken.insert(merged.name.clone());
        }
        let fields = changed_fields(&tool.input(), &merged);
        let op_id_changed = tool.source.op_id() != Some(op.op_id().as_str());
        if fields.is_empty() && !op_id_changed {
            out.unchanged.push(tool);
        } else {
            out.changed.push(Change {
                current: tool,
                merged,
                fields,
                op,
            });
        }
    }
    // Removed tools keep their names unless the user deletes them.
    for t in &out.removed {
        taken.insert(t.name.clone());
    }
    for op in &imported.operations {
        if !matched_keys.contains(op.key.as_str()) {
            let name = unique_name(op.tool.name.clone(), &mut taken);
            out.added.push((op, name));
        }
    }
    out
}

#[derive(Deserialize)]
struct PreviewRequest {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    url: Option<String>,
    /// `None` → the base URL used last time; `""` → the spec's servers.
    #[serde(default)]
    base_url: Option<String>,
    /// `None` → the auth config used last time; `""` → no auth.
    #[serde(default)]
    auth_ref: Option<String>,
}

struct Inputs {
    spec: SpecRecord,
    content: String,
    source_url: Option<String>,
    fetched: bool,
    base_url: Option<String>,
    auth_ref: Option<Uuid>,
}

async fn resolve_inputs(
    s: &AppState,
    project_id: Uuid,
    spec_id: Uuid,
    content: Option<String>,
    url: Option<String>,
    base_url: Option<String>,
    auth_ref: Option<String>,
) -> Result<Inputs, ApiError> {
    let spec = s.store.get_spec(project_id, spec_id).await?;
    let (content, source_url, fetched) = match (content, url) {
        (Some(c), _) if !c.trim().is_empty() => (c, None, false),
        (_, Some(u)) if !u.trim().is_empty() => {
            (fetch_spec(u.trim()).await?, Some(u.trim().to_owned()), true)
        }
        // Neither: re-run against the stored spec (e.g. to change base URL or auth).
        _ => (
            s.store.get_spec_raw(project_id, spec_id).await?,
            spec.source_url.clone(),
            false,
        ),
    };
    let stored_base = spec
        .options
        .get("base_url")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let base_url = match base_url {
        None => stored_base,
        Some(b) if b.trim().is_empty() => None,
        Some(b) => Some(b.trim().to_owned()),
    };
    let auth_ref = match auth_ref {
        None => spec
            .options
            .get("auth_ref")
            .and_then(Value::as_str)
            .and_then(|a| Uuid::parse_str(a).ok()),
        Some(a) if a.is_empty() => None,
        Some(a) => Some(Uuid::parse_str(&a).map_err(|_| {
            ApiError::Store(ValidationErrors::single("auth_ref", "not a valid id").into())
        })?),
    };
    Ok(Inputs {
        spec,
        content,
        source_url,
        fetched,
        base_url,
        auth_ref,
    })
}

async fn other_tool_names(
    s: &AppState,
    project_id: Uuid,
    spec_id: Uuid,
) -> Result<(Vec<ToolDef>, HashSet<String>), ApiError> {
    let all = s.store.list_tools(project_id).await?;
    let (mine, others): (Vec<_>, Vec<_>) = all
        .into_iter()
        .partition(|t| t.source.spec_id() == Some(spec_id));
    Ok((mine, others.into_iter().map(|t| t.name).collect()))
}

async fn preview(
    State(s): State<AppState>,
    ApiPath((project_id, spec_id)): ApiPath<(Uuid, Uuid)>,
    ApiJson(req): ApiJson<PreviewRequest>,
) -> Result<Json<Value>, ApiError> {
    let inp = resolve_inputs(
        &s,
        project_id,
        spec_id,
        req.content,
        req.url,
        req.base_url,
        req.auth_ref,
    )
    .await?;
    let imported = run_import(&inp.content, inp.base_url.clone(), inp.source_url.clone())?;
    let (mine, others) = other_tool_names(&s, project_id, spec_id).await?;
    let p = plan(&imported, mine, others, inp.auth_ref);

    Ok(Json(json!({
        "spec": inp.spec,
        "info": imported.info,
        "base_url": inp.base_url,
        "auth_ref": inp.auth_ref,
        "changed": p.changed.iter().map(|c| json!({
            "tool_id": c.current.id,
            "name": c.current.name,
            "new_name": c.merged.name,
            "key": c.op.key,
            "fields": c.fields,
            "kept": c.current.overrides.fields,
        })).collect::<Vec<_>>(),
        "unchanged": p.unchanged.len(),
        "blocked": p.blocked.iter().map(|(t, op)| json!({
            "tool_id": t.id, "name": t.name, "key": op.key, "warnings": op.warnings,
        })).collect::<Vec<_>>(),
        "added": p.added.iter().map(|(op, name)| {
            let mut sum = serde_json::to_value(op.summary()).expect("summary serializes");
            sum["name"] = json!(name);
            sum
        }).collect::<Vec<_>>(),
        "removed": p.removed.iter().map(|t| json!({
            "tool_id": t.id, "name": t.name, "op_id": t.source.op_id(),
        })).collect::<Vec<_>>(),
        "content": inp.fetched.then_some(&inp.content),
        "source_url": inp.source_url,
    })))
}

#[derive(Deserialize)]
struct CommitRequest {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    source_url: Option<String>,
    #[serde(default)]
    filename: Option<String>,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    auth_ref: Option<String>,
    /// Keys of new operations to add.
    #[serde(default)]
    add: Vec<String>,
    /// Ids of tools whose operation is gone, to delete.
    #[serde(default)]
    remove: Vec<Uuid>,
}

async fn commit(
    State(s): State<AppState>,
    ApiPath((project_id, spec_id)): ApiPath<(Uuid, Uuid)>,
    ApiJson(req): ApiJson<CommitRequest>,
) -> Result<Json<Value>, ApiError> {
    let uploaded = req.content.as_ref().is_some_and(|c| !c.trim().is_empty());
    let mut inp = resolve_inputs(
        &s,
        project_id,
        spec_id,
        req.content,
        None,
        req.base_url,
        req.auth_ref,
    )
    .await?;
    if uploaded {
        // Content fetched in the preview comes back here with its URL.
        inp.source_url = req.source_url.clone();
    }
    let imported = run_import(&inp.content, inp.base_url.clone(), inp.source_url.clone())?;
    let (mine, others) = other_tool_names(&s, project_id, spec_id).await?;
    let p = plan(&imported, mine, others, inp.auth_ref);

    let mut errs = ValidationErrors::default();
    let added: HashMap<&str, (&BuiltOperation, &String)> = p
        .added
        .iter()
        .map(|(op, name)| (op.key.as_str(), (*op, name)))
        .collect();
    let mut inserts = Vec::new();
    for (i, key) in req.add.iter().enumerate() {
        match added.get(key.as_str()) {
            Some((op, _)) if !op.supported => errs.push(
                format!("add[{i}]"),
                format!("`{key}` can't be imported: {}", op.warnings.join(" ")),
            ),
            Some((op, name)) => {
                let mut t = op.tool.clone();
                t.name = (*name).clone();
                if op.needs_auth || !imported.operations.iter().any(|o| o.needs_auth) {
                    let ToolKind::Http(h) = &mut t.kind;
                    h.auth_ref = inp.auth_ref;
                }
                inserts.push((t, op.op_id()));
            }
            None => errs.push(
                format!("add[{i}]"),
                format!("`{key}` isn't a new operation in this spec"),
            ),
        }
    }
    let removable: HashSet<Uuid> = p.removed.iter().map(|t| t.id).collect();
    for (i, id) in req.remove.iter().enumerate() {
        if !removable.contains(id) {
            errs.push(
                format!("remove[{i}]"),
                "only tools whose operation left the spec can be removed here",
            );
        }
    }
    errs.into_result().map_err(|e| ApiError::Store(e.into()))?;

    let updates: Vec<(Uuid, ToolInput, String)> = p
        .changed
        .iter()
        .map(|c| (c.current.id, c.merged.clone(), c.op.op_id()))
        .collect();
    let spec = NewSpec {
        kind: imported.info.kind.clone(),
        title: imported.info.title.clone(),
        version: imported.info.api_version.clone(),
        source_url: inp.source_url.clone(),
        filename: if uploaded {
            req.filename
        } else {
            inp.spec.filename.clone()
        },
        raw: inp.content,
        options: json!({ "base_url": inp.base_url, "auth_ref": inp.auth_ref }),
    };
    let (record, result) = s
        .store
        .apply_reimport(project_id, spec_id, &spec, &updates, &inserts, &req.remove)
        .await?;
    s.changes.notify(project_id);
    tracing::info!(
        project = %project_id, spec = %spec_id,
        updated = result.updated.len(), created = result.created.len(), deleted = result.deleted.len(),
        "re-imported spec"
    );
    Ok(Json(json!({ "spec": record, "result": result })))
}
