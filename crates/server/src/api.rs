//! REST API under `/api`: CRUD for projects, tools and auth configs.
//!
//! Errors are JSON: `{"error": {"code", "message", "fields"?}}` with
//! 400 (malformed request), 404, 409 (conflict), 422 (validation) or 500.

use axum::extract::rejection::{JsonRejection, PathRejection};
use axum::extract::{FromRequest, FromRequestParts, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use forge_core::{AuthConfig, AuthConfigInput, Project, ProjectInput, ToolDef, ToolInput};
use forge_exec_http::ExecError;
use forge_store::StoreError;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/projects", get(list_projects).post(create_project))
        .route(
            "/projects/{project_id}",
            get(get_project).put(update_project).delete(delete_project),
        )
        .route(
            "/projects/{project_id}/settings",
            axum::routing::put(update_settings),
        )
        .route(
            "/projects/{project_id}/tools",
            get(list_tools).post(create_tool),
        )
        .route(
            "/projects/{project_id}/tools/{tool_id}",
            get(get_tool).put(update_tool).delete(delete_tool),
        )
        .route(
            "/projects/{project_id}/tools/{tool_id}/mcp",
            get(get_tool_mcp_view),
        )
        .route(
            "/projects/{project_id}/tools/{tool_id}/test",
            axum::routing::post(test_tool),
        )
        .route(
            "/projects/{project_id}/tools/{tool_id}/overrides/reset",
            axum::routing::post(reset_overrides),
        )
        .route("/projects/{project_id}/export", get(export_bundle))
        .route(
            "/projects/{project_id}/import/bundle",
            axum::routing::post(import_bundle)
                .layer(axum::extract::DefaultBodyLimit::max(16 * 1024 * 1024)),
        )
        .route("/info", get(info))
        .route(
            "/projects/{project_id}/auth-configs",
            get(list_auth_configs).post(create_auth_config),
        )
        .route(
            "/projects/{project_id}/auth-configs/{auth_id}",
            get(get_auth_config)
                .put(update_auth_config)
                .delete(delete_auth_config),
        )
        .fallback(|| async { ApiError::Store(StoreError::NotFound("route")) })
}

// ---- errors & extractors ----------------------------------------------------

#[derive(Debug)]
pub enum ApiError {
    BadRequest(String),
    Store(StoreError),
}

impl From<StoreError> for ApiError {
    fn from(e: StoreError) -> Self {
        ApiError::Store(e)
    }
}

impl From<JsonRejection> for ApiError {
    fn from(r: JsonRejection) -> Self {
        ApiError::BadRequest(r.body_text())
    }
}

impl From<PathRejection> for ApiError {
    fn from(r: PathRejection) -> Self {
        ApiError::BadRequest(r.body_text())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, body) = match self {
            ApiError::BadRequest(msg) => (
                StatusCode::BAD_REQUEST,
                json!({ "code": "bad_request", "message": msg }),
            ),
            ApiError::Store(StoreError::NotFound(what)) => (
                StatusCode::NOT_FOUND,
                json!({ "code": "not_found", "message": format!("{what} not found") }),
            ),
            ApiError::Store(StoreError::Conflict(msg)) => (
                StatusCode::CONFLICT,
                json!({ "code": "conflict", "message": msg }),
            ),
            ApiError::Store(StoreError::Validation(errs)) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                json!({
                    "code": "validation",
                    "message": errs.to_string(),
                    "fields": errs.errors,
                }),
            ),
            ApiError::Store(e) => {
                tracing::error!(error = %e, "internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    json!({ "code": "internal", "message": "internal error; see server logs" }),
                )
            }
        };
        (status, Json(json!({ "error": body }))).into_response()
    }
}

/// `Json` with rejections mapped to our error shape.
#[derive(FromRequest)]
#[from_request(via(axum::Json), rejection(ApiError))]
pub struct ApiJson<T>(pub T);

/// `Path` with rejections mapped to our error shape.
#[derive(FromRequestParts)]
#[from_request(via(axum::extract::Path), rejection(ApiError))]
pub struct ApiPath<T>(pub T);

type ApiResult<T> = Result<T, ApiError>;

// ---- projects ---------------------------------------------------------------

async fn list_projects(State(s): State<AppState>) -> ApiResult<Json<Vec<Project>>> {
    Ok(Json(s.store.list_projects().await?))
}

async fn create_project(
    State(s): State<AppState>,
    ApiJson(input): ApiJson<ProjectInput>,
) -> ApiResult<(StatusCode, Json<Project>)> {
    Ok((
        StatusCode::CREATED,
        Json(s.store.create_project(&input).await?),
    ))
}

async fn get_project(
    State(s): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
) -> ApiResult<Json<Project>> {
    Ok(Json(s.store.get_project(id).await?))
}

async fn update_project(
    State(s): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(input): ApiJson<ProjectInput>,
) -> ApiResult<Json<Project>> {
    Ok(Json(s.store.update_project(id, &input).await?))
}

async fn delete_project(
    State(s): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
) -> ApiResult<StatusCode> {
    s.store.delete_project(id).await?;
    s.changes.notify(id);
    Ok(StatusCode::NO_CONTENT)
}

// ---- tools ------------------------------------------------------------------

async fn list_tools(
    State(s): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
) -> ApiResult<Json<Vec<ToolDef>>> {
    Ok(Json(s.store.list_tools(project_id).await?))
}

async fn create_tool(
    State(s): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
    ApiJson(input): ApiJson<ToolInput>,
) -> ApiResult<(StatusCode, Json<ToolDef>)> {
    check_shape(&input)?;
    let tool = s.store.create_tool(project_id, &input).await?;
    s.changes.notify(project_id);
    Ok((StatusCode::CREATED, Json(tool)))
}

async fn get_tool(
    State(s): State<AppState>,
    ApiPath((project_id, id)): ApiPath<(Uuid, Uuid)>,
) -> ApiResult<Json<ToolDef>> {
    Ok(Json(s.store.get_tool(project_id, id).await?))
}

async fn update_tool(
    State(s): State<AppState>,
    ApiPath((project_id, id)): ApiPath<(Uuid, Uuid)>,
    ApiJson(input): ApiJson<ToolInput>,
) -> ApiResult<Json<ToolDef>> {
    check_shape(&input)?;
    let tool = s.store.update_tool(project_id, id, &input).await?;
    s.changes.notify(project_id);
    Ok(Json(tool))
}

async fn delete_tool(
    State(s): State<AppState>,
    ApiPath((project_id, id)): ApiPath<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    s.store.delete_tool(project_id, id).await?;
    s.changes.notify(project_id);
    Ok(StatusCode::NO_CONTENT)
}

// ---- auth configs -------------------------------------------------------------

async fn list_auth_configs(
    State(s): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
) -> ApiResult<Json<Vec<AuthConfig>>> {
    Ok(Json(s.store.list_auth_configs(project_id).await?))
}

async fn create_auth_config(
    State(s): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
    ApiJson(input): ApiJson<AuthConfigInput>,
) -> ApiResult<(StatusCode, Json<AuthConfig>)> {
    Ok((
        StatusCode::CREATED,
        Json(s.store.create_auth_config(project_id, &input).await?),
    ))
}

async fn get_auth_config(
    State(s): State<AppState>,
    ApiPath((project_id, id)): ApiPath<(Uuid, Uuid)>,
) -> ApiResult<Json<AuthConfig>> {
    Ok(Json(s.store.get_auth_config(project_id, id).await?))
}

async fn update_auth_config(
    State(s): State<AppState>,
    ApiPath((project_id, id)): ApiPath<(Uuid, Uuid)>,
    ApiJson(input): ApiJson<AuthConfigInput>,
) -> ApiResult<Json<AuthConfig>> {
    Ok(Json(
        s.store.update_auth_config(project_id, id, &input).await?,
    ))
}

async fn delete_auth_config(
    State(s): State<AppState>,
    ApiPath((project_id, id)): ApiPath<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    s.store.delete_auth_config(project_id, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- info, MCP view, test console ---------------------------------------------

/// What the UI needs to render connection snippets.
async fn info(State(s): State<AppState>) -> Json<Value> {
    let exe = std::env::current_exe()
        .ok()
        .map(|p| p.display().to_string());
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "exe": exe,
        "data_dir": s.data_dir.as_ref().map(|d| d.display().to_string()),
        "auth_required": s.security.admin_token.is_some(),
    }))
}

/// The tool exactly as MCP clients see it (derived input schema, annotations).
async fn get_tool_mcp_view(
    State(s): State<AppState>,
    ApiPath((project_id, id)): ApiPath<(Uuid, Uuid)>,
) -> ApiResult<Json<rmcp::model::Tool>> {
    let tool = s.store.get_tool(project_id, id).await?;
    Ok(Json(forge_mcp::to_mcp_tool(&tool)))
}

#[derive(Deserialize)]
struct TestRequest {
    #[serde(default)]
    arguments: Map<String, Value>,
}

/// Runs a saved tool (even if disabled) and reports the request sent, the raw
/// upstream response and exactly what an agent would receive.
async fn test_tool(
    State(s): State<AppState>,
    ApiPath((project_id, id)): ApiPath<(Uuid, Uuid)>,
    ApiJson(req): ApiJson<TestRequest>,
) -> ApiResult<Json<Value>> {
    let tool = s.store.get_tool(project_id, id).await?;
    let settings = s.store.get_project(project_id).await?.settings;
    let run = forge_mcp::run_tool(&s.store, &s.exec, &tool, &req.arguments, &settings).await?;
    let (request, response, error) = match &run.outcome {
        Ok(o) => (
            json!({ "method": o.method, "url": o.url }),
            json!({
                "status": o.status,
                "status_line": o.status_line(),
                "content_type": o.content_type,
                "body": o.body,
                "truncated": o.truncated,
            }),
            Value::Null,
        ),
        Err(e) => {
            let err = match e {
                ExecError::InvalidArgs(v) => json!({
                    "kind": "invalid_args", "message": v.to_string(), "fields": v.errors,
                }),
                ExecError::Config(m) => json!({ "kind": "config", "message": m }),
                ExecError::Request(m) => json!({ "kind": "request", "message": m }),
            };
            (Value::Null, Value::Null, err)
        }
    };
    Ok(Json(json!({
        "request": request,
        "response": response,
        "error": error,
        "agent_view": { "is_error": run.is_error, "text": run.text },
        // The agent-facing text when shaping changed it (jq, nulls, size cap).
        "shaped": run.shaped,
    })))
}

#[derive(Deserialize, Default)]
struct ResetRequest {
    /// Fields to stop keeping; all of them when absent.
    #[serde(default)]
    fields: Option<Vec<String>>,
}

/// Stops keeping the user's version of some fields on re-import.
async fn reset_overrides(
    State(s): State<AppState>,
    ApiPath((project_id, id)): ApiPath<(Uuid, Uuid)>,
    ApiJson(req): ApiJson<ResetRequest>,
) -> ApiResult<Json<ToolDef>> {
    Ok(Json(
        s.store
            .reset_overrides(project_id, id, req.fields.as_deref())
            .await?,
    ))
}

/// `tools.yaml` download for a project.
async fn export_bundle(
    State(s): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
) -> ApiResult<axum::response::Response> {
    let project = s.store.get_project(project_id).await?;
    let yaml = crate::bundle::export_project(&s.store, project_id).await?;
    let disposition = format!("attachment; filename=\"{}-tools.yaml\"", project.slug);
    Ok((
        [
            (
                axum::http::header::CONTENT_TYPE,
                "application/yaml; charset=utf-8".to_owned(),
            ),
            (axum::http::header::CONTENT_DISPOSITION, disposition),
        ],
        yaml,
    )
        .into_response())
}

#[derive(Deserialize)]
struct BundleImportRequest {
    content: String,
    #[serde(default)]
    on_conflict: forge_store::ConflictPolicy,
}

async fn import_bundle(
    State(s): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
    ApiJson(req): ApiJson<BundleImportRequest>,
) -> ApiResult<Json<forge_store::BundleReport>> {
    let bundle =
        crate::bundle::parse_bundle(&req.content).map_err(|e| ApiError::Store(e.into()))?;
    for (i, (t, _)) in bundle.tools.iter().enumerate() {
        check_shape(t).map_err(|e| match e {
            ApiError::Store(StoreError::Validation(mut v)) => {
                for f in &mut v.errors {
                    f.field = format!("tools[{i}].{}", f.field);
                }
                ApiError::Store(StoreError::Validation(v))
            }
            other => other,
        })?;
    }
    let report = s
        .store
        .import_bundle(
            project_id,
            &bundle.auth_configs,
            &bundle.tools,
            req.on_conflict,
        )
        .await?;
    s.changes.notify(project_id);
    Ok(Json(report))
}

/// A `response_shape` must compile before it's saved.
fn check_shape(input: &ToolInput) -> ApiResult<()> {
    if let Some(expr) = input
        .response_shape
        .as_deref()
        .filter(|e| !e.trim().is_empty())
    {
        forge_exec_http::shape::check(expr).map_err(|m| {
            ApiError::Store(forge_core::ValidationErrors::single("response_shape", m).into())
        })?;
    }
    Ok(())
}

/// Replaces the project's curation and shaping settings.
async fn update_settings(
    State(s): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(settings): ApiJson<forge_core::ProjectSettings>,
) -> ApiResult<Json<Project>> {
    let project = s.store.update_project_settings(id, &settings).await?;
    // Filters and tool_search change what agents see.
    s.changes.notify(id);
    Ok(Json(project))
}
