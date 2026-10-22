//! OpenAPI 3.x / Swagger 2.0 import: preview (parse + propose tools) and commit (store selected).
//!
//! The spec text travels with both calls, so the server keeps no import state.
//! A URL is fetched only when the user asks for it in the preview call.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use axum::extract::{DefaultBodyLimit, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use forge_core::{ToolKind, ValidationErrors};
use forge_importers::openapi3::{self, BuiltOperation, Imported};
use forge_importers::{import_any, parse_document, unique_name, ImportError};
use forge_store::{NewSpec, SpecRecord, StoreError};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::api::{ApiError, ApiJson, ApiPath};
use crate::AppState;

/// Specs like GitHub's are ~14 MB; leave room for JSON escaping.
pub(crate) const MAX_SPEC_BYTES: usize = 48 * 1024 * 1024;
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/projects/{project_id}/import/openapi/preview",
            post(preview).layer(DefaultBodyLimit::max(MAX_SPEC_BYTES)),
        )
        .route(
            "/projects/{project_id}/import/openapi",
            post(commit).layer(DefaultBodyLimit::max(MAX_SPEC_BYTES)),
        )
        .route("/projects/{project_id}/specs", get(list_specs))
}

#[derive(Deserialize)]
struct PreviewRequest {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    base_url: Option<String>,
}

async fn preview(
    State(s): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
    ApiJson(req): ApiJson<PreviewRequest>,
) -> Result<Json<Value>, ApiError> {
    s.store.get_project(project_id).await?;
    let (content, source_url) = match (req.content, req.url) {
        (Some(c), _) if !c.trim().is_empty() => (c, None),
        (_, Some(u)) if !u.trim().is_empty() => {
            (fetch_spec(u.trim()).await?, Some(u.trim().to_owned()))
        }
        _ => {
            return Err(ApiError::BadRequest(
                "Send the spec text or a URL to fetch it from.".into(),
            ))
        }
    };
    let imported = run_import(&content, req.base_url, source_url.clone())?;
    let existing = s.store.tool_names(project_id).await?;
    let names = assign_names(&imported.operations, &existing);
    let operations: Vec<Value> = imported
        .operations
        .iter()
        .map(|o| {
            let mut sum = serde_json::to_value(o.summary()).expect("summary serializes");
            sum["name"] = json!(names[&o.key]);
            sum
        })
        .collect();
    Ok(Json(json!({
        "info": imported.info,
        "operations": operations,
        "any_auth": imported.operations.iter().any(|o| o.needs_auth),
        // Echoed back only when fetched, so the client can send it with the commit.
        "content": source_url.as_ref().map(|_| &content),
        "source_url": source_url,
    })))
}

#[derive(Deserialize)]
struct SelectedOp {
    key: String,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize)]
struct CommitRequest {
    content: String,
    #[serde(default)]
    source_url: Option<String>,
    #[serde(default)]
    filename: Option<String>,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    auth_ref: Option<Uuid>,
    operations: Vec<SelectedOp>,
}

async fn commit(
    State(s): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
    ApiJson(req): ApiJson<CommitRequest>,
) -> Result<Json<Value>, ApiError> {
    if req.operations.is_empty() {
        return Err(ApiError::BadRequest(
            "Select at least one operation to import.".into(),
        ));
    }
    let base_url_opt = req.base_url.clone().filter(|b| !b.trim().is_empty());
    let imported = run_import(&req.content, req.base_url, req.source_url.clone())?;
    let existing = s.store.tool_names(project_id).await?;
    let names = assign_names(&imported.operations, &existing);
    let by_key: HashMap<&str, &BuiltOperation> = imported
        .operations
        .iter()
        .map(|o| (o.key.as_str(), o))
        .collect();
    // Without any security info in the spec, the chosen auth applies to everything.
    let any_auth = imported.operations.iter().any(|o| o.needs_auth);

    let mut errs = ValidationErrors::default();
    let mut tools = Vec::new();
    let mut seen = HashSet::new();
    for (i, sel) in req.operations.iter().enumerate() {
        let field = format!("operations[{i}]");
        let Some(op) = by_key.get(sel.key.as_str()) else {
            errs.push(
                field,
                format!("`{}` isn't an operation in this spec", sel.key),
            );
            continue;
        };
        if !seen.insert(sel.key.as_str()) {
            errs.push(field, format!("`{}` is selected twice", sel.key));
            continue;
        }
        if !op.supported {
            errs.push(
                field,
                format!("`{}` can't be imported: {}", sel.key, op.warnings.join(" ")),
            );
            continue;
        }
        let mut tool = op.tool.clone();
        tool.name = sel.name.clone().unwrap_or_else(|| names[&op.key].clone());
        if op.needs_auth || !any_auth {
            let ToolKind::Http(h) = &mut tool.kind;
            h.auth_ref = req.auth_ref;
        }
        tools.push((tool, op.op_id()));
    }
    errs.into_result().map_err(|e| ApiError::Store(e.into()))?;

    let spec = NewSpec {
        kind: imported.info.kind.clone(),
        title: imported.info.title.clone(),
        version: imported.info.api_version.clone(),
        source_url: req.source_url,
        filename: req.filename,
        raw: req.content,
        options: json!({ "base_url": base_url_opt, "auth_ref": req.auth_ref }),
    };
    let (record, created) = s.store.import_tools(project_id, &spec, &tools).await?;
    s.changes.notify(project_id);
    tracing::info!(project = %project_id, spec = %record.id, kind = %record.kind, tools = created.len(), "imported spec");
    Ok(Json(json!({ "spec": record, "created": created })))
}

async fn list_specs(
    State(s): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
) -> Result<Json<Vec<SpecRecord>>, ApiError> {
    Ok(Json(s.store.list_specs(project_id).await?))
}

pub(crate) fn run_import(
    content: &str,
    base_url: Option<String>,
    source_url: Option<String>,
) -> Result<Imported, ApiError> {
    let doc = parse_document(content).map_err(import_error)?;
    import_any(
        &doc,
        &openapi3::Options {
            base_url,
            source_url,
        },
    )
    .map_err(import_error)
}

pub(crate) fn import_error(e: ImportError) -> ApiError {
    ApiError::Store(StoreError::Validation(ValidationErrors::single(
        "content",
        e.to_string(),
    )))
}

/// Final tool names: the importer's names, renamed (`_2`, …) where they would
/// clash with tools already in the project. Deterministic, so the preview
/// shows exactly the names the commit will create.
fn assign_names(ops: &[BuiltOperation], existing: &[String]) -> HashMap<String, String> {
    let mut taken: HashSet<String> = existing.iter().cloned().collect();
    let spec_names: HashSet<&str> = ops.iter().map(|o| o.tool.name.as_str()).collect();
    // Reserve the spec's own names first so renamed ones can't steal them.
    for n in &spec_names {
        if !existing.iter().any(|e| e == n) {
            taken.insert((*n).to_owned());
        }
    }
    ops.iter()
        .map(|o| {
            let name = if existing.iter().any(|e| e == &o.tool.name) {
                unique_name(o.tool.name.clone(), &mut taken)
            } else {
                o.tool.name.clone()
            };
            (o.key.clone(), name)
        })
        .collect()
}

/// Fetches a spec from a URL the user typed. http(s) only, size- and time-capped.
pub(crate) async fn fetch_spec(url: &str) -> Result<String, ApiError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|e| ApiError::BadRequest(format!("That URL isn't valid: {e}")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(ApiError::BadRequest(
            "Only http and https URLs can be fetched.".into(),
        ));
    }
    tracing::info!(url = %parsed, "fetching spec (user request)");
    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(5))
        .user_agent(concat!("mcp-forge/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("static client config");
    let fail = |e: reqwest::Error| {
        ApiError::BadRequest(format!("Couldn't fetch the spec: {}", e.without_url()))
    };
    let mut resp = client.get(parsed).send().await.map_err(fail)?;
    if !resp.status().is_success() {
        return Err(ApiError::BadRequest(format!(
            "Couldn't fetch the spec: the server answered {}.",
            resp.status()
        )));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(fail)? {
        if bytes.len() + chunk.len() > MAX_SPEC_BYTES {
            return Err(ApiError::BadRequest(
                "That spec is larger than 48 MB.".into(),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes)
        .map_err(|_| ApiError::BadRequest("The fetched spec isn't UTF-8 text.".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_importers::openapi3::import;
    use forge_importers::parse_document;

    #[test]
    fn assign_names_avoids_existing_and_keeps_spec_names() {
        let doc = parse_document(
            r#"{"openapi":"3.0.0","info":{"title":"t","version":"1"},"servers":[{"url":"https://x.io"}],
                "paths":{"/a":{"get":{"operationId":"a"}},"/a2":{"get":{"operationId":"a_2"}},"/b":{"get":{"operationId":"b"}}}}"#,
        )
        .unwrap();
        let imported = import(&doc, &openapi3::Options::default()).unwrap();
        let names = assign_names(&imported.operations, &["a".into()]);
        // `a` clashes with an existing tool; `a_2` belongs to the spec, so it becomes `a_3`.
        assert_eq!(names["GET /a"], "a_3");
        assert_eq!(names["GET /a2"], "a_2");
        assert_eq!(names["GET /b"], "b");
    }
}
