//! REST API under `/api`: CRUD for projects, tools, auth configs and DB connections.
//!
//! Errors are JSON: `{"error": {"code", "message", "fields"?}}` with
//! 400 (malformed request), 404, 409 (conflict), 422 (validation) or 500.

use axum::extract::rejection::{JsonRejection, PathRejection};
use axum::extract::{FromRequest, FromRequestParts, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use forge_core::{
    AuthConfig, AuthConfigInput, DbConnection, DbConnectionInput, Project, ProjectInput, SqlMode,
    ToolDef, ToolInput, ToolKind,
};
use forge_exec_http::ExecError;
use forge_exec_sql::SqlError;
use forge_mcp::RunOutcome;
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
        .route(
            "/projects/{project_id}/db-connections",
            get(list_connections).post(create_connection),
        )
        .route(
            "/projects/{project_id}/db-connections/{conn_id}",
            get(get_connection)
                .put(update_connection)
                .delete(delete_connection),
        )
        .route(
            "/projects/{project_id}/db-connections/{conn_id}/test",
            axum::routing::post(test_connection),
        )
        .route("/projects/{project_id}/audit", get(list_audit))
        .route(
            "/projects/{project_id}/db-connections/{conn_id}/exploration-tools",
            axum::routing::post(add_exploration_tools),
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
    check_sql(&s, project_id, &input).await?;
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
    check_sql(&s, project_id, &input).await?;
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

// ---- DB connections -------------------------------------------------------------

async fn list_connections(
    State(s): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
) -> ApiResult<Json<Vec<DbConnection>>> {
    Ok(Json(s.store.list_connections(project_id).await?))
}

/// Connection payload plus the explicit opt-in that `full_access` needs.
#[derive(Deserialize)]
struct ConnectionRequest {
    #[serde(flatten)]
    input: DbConnectionInput,
    #[serde(default)]
    confirm_full_access: bool,
}

/// Switching a connection to `full_access` (agents may run their own
/// writes) must be confirmed explicitly; the UI asks with a warning.
fn check_full_access(
    req: &ConnectionRequest,
    before: Option<forge_core::AccessMode>,
) -> ApiResult<()> {
    use forge_core::AccessMode::FullAccess;
    if req.input.access_mode == FullAccess && before != Some(FullAccess) && !req.confirm_full_access
    {
        return Err(ApiError::Store(
            forge_core::ValidationErrors::single(
                "access_mode",
                "full_access lets agents change and delete data with SQL they write; confirm it with confirm_full_access: true",
            )
            .into(),
        ));
    }
    Ok(())
}

async fn create_connection(
    State(s): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
    ApiJson(req): ApiJson<ConnectionRequest>,
) -> ApiResult<(StatusCode, Json<DbConnection>)> {
    check_full_access(&req, None)?;
    Ok((
        StatusCode::CREATED,
        Json(s.store.create_connection(project_id, &req.input).await?),
    ))
}

async fn get_connection(
    State(s): State<AppState>,
    ApiPath((project_id, id)): ApiPath<(Uuid, Uuid)>,
) -> ApiResult<Json<DbConnection>> {
    Ok(Json(s.store.get_connection(project_id, id).await?))
}

async fn update_connection(
    State(s): State<AppState>,
    ApiPath((project_id, id)): ApiPath<(Uuid, Uuid)>,
    ApiJson(req): ApiJson<ConnectionRequest>,
) -> ApiResult<Json<DbConnection>> {
    let before = s.store.get_connection(project_id, id).await?.access_mode;
    check_full_access(&req, Some(before))?;
    let conn = s
        .store
        .update_connection(project_id, id, &req.input)
        .await?;
    // Tool descriptions, schemas (dry_run) and annotations depend on it.
    s.changes.notify(project_id);
    s.exec.sql.forget(id).await;
    Ok(Json(conn))
}

async fn delete_connection(
    State(s): State<AppState>,
    ApiPath((project_id, id)): ApiPath<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    s.store.delete_connection(project_id, id).await?;
    s.exec.sql.forget(id).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Connects with the saved settings and reports version, user and whether
/// the role can write.
async fn test_connection(
    State(s): State<AppState>,
    ApiPath((project_id, id)): ApiPath<(Uuid, Uuid)>,
) -> ApiResult<Json<forge_exec_sql::ConnectionReport>> {
    let conn = s.store.get_connection(project_id, id).await?;
    Ok(Json(s.exec.sql.test_connection(&conn).await))
}

#[derive(Deserialize)]
struct AuditQuery {
    #[serde(default)]
    limit: Option<u32>,
    /// Only entries older than this (paging).
    #[serde(default)]
    before: Option<chrono::DateTime<chrono::Utc>>,
}

/// SQL write attempts, newest first.
async fn list_audit(
    State(s): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
    axum::extract::Query(q): axum::extract::Query<AuditQuery>,
) -> ApiResult<Json<Vec<forge_core::AuditEntry>>> {
    Ok(Json(
        s.store
            .list_audit(project_id, q.limit.unwrap_or(100), q.before)
            .await?,
    ))
}

#[derive(Deserialize)]
struct ExplorationRequest {
    /// Any of `list_tables`, `describe_table`, `run_sql`.
    modes: Vec<String>,
}

/// Opt-in: creates the chosen exploration tools for a connection, named
/// `{connection}_{mode}`. Modes the connection already has a tool for are
/// skipped. Descriptions stay empty so they're generated from the
/// connection's current settings.
async fn add_exploration_tools(
    State(s): State<AppState>,
    ApiPath((project_id, id)): ApiPath<(Uuid, Uuid)>,
    ApiJson(req): ApiJson<ExplorationRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let conn = s.store.get_connection(project_id, id).await?;
    let tools = s.store.list_tools(project_id).await?;
    let mut created = Vec::new();
    let mut existing = Vec::new();
    let mut taken: std::collections::HashSet<String> =
        tools.iter().map(|t| t.name.clone()).collect();
    for (i, m) in req.modes.iter().enumerate() {
        let mode = match m.as_str() {
            "list_tables" => SqlMode::ListTables,
            "describe_table" => SqlMode::DescribeTable,
            "run_sql" => SqlMode::RunSql,
            other => {
                return Err(ApiError::Store(
                    forge_core::ValidationErrors::single(
                        format!("modes[{i}]"),
                        format!(
                            "unknown mode `{other}`; use list_tables, describe_table or run_sql"
                        ),
                    )
                    .into(),
                ))
            }
        };
        let have = tools.iter().find(
            |t| matches!(&t.kind, ToolKind::Sql(sp) if sp.connection_id == id && sp.mode == mode),
        );
        if let Some(t) = have {
            existing.push(t.name.clone());
            continue;
        }
        let suffix = format!("_{}", mode.as_str());
        let base: String = conn
            .name
            .chars()
            .take(forge_core::MAX_NAME_LEN - suffix.len() - 3)
            .collect::<String>()
            + &suffix;
        let mut name = base.clone();
        let mut n = 2;
        while taken.contains(&name) {
            name = format!("{base}_{n}");
            n += 1;
        }
        taken.insert(name.clone());
        let input = ToolInput {
            name,
            description: String::new(),
            kind: ToolKind::Sql(forge_core::SqlSpec {
                connection_id: id,
                mode,
            }),
            enabled: true,
            tags: vec!["database".into()],
            response_shape: None,
        };
        created.push(s.store.create_tool(project_id, &input).await?);
    }
    if !created.is_empty() {
        s.changes.notify(project_id);
    }
    Ok((
        StatusCode::CREATED,
        Json(json!({ "created": created, "existing": existing })),
    ))
}

/// A SQL tool's statement must pass the read-only check for its
/// connection's dialect before it's saved.
pub(crate) async fn check_sql(s: &AppState, project_id: Uuid, input: &ToolInput) -> ApiResult<()> {
    let ToolKind::Sql(spec) = &input.kind else {
        return Ok(());
    };
    // A missing connection is reported by the store (422 on kind.connection_id).
    let Ok(conn) = s.store.get_connection(project_id, spec.connection_id).await else {
        return Ok(());
    };
    forge_exec_sql::check_query_tool(spec, &conn.input())
        .map_err(|e| ApiError::Store(StoreError::Validation(e)))
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
    let conn = match tool.kind.connection_ref() {
        Some(cid) => s.store.get_connection(project_id, cid).await.ok(),
        None => None,
    };
    Ok(Json(forge_mcp::to_mcp_tool(&tool, conn.as_ref())))
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
    let run = forge_mcp::run_tool(
        &s.store,
        &s.exec,
        &tool,
        &req.arguments,
        &settings,
        "console",
    )
    .await?;
    let (request, response, error) = match &run.outcome {
        RunOutcome::Sql(Ok(o)) => (
            json!({ "sql": o.sql, "params": o.params }),
            json!({
                "columns": o.columns,
                "rows": o.rows,
                "row_count": o.rows.len(),
                "truncated": o.truncated,
                "masked_columns": o.masked_columns,
                "elapsed_ms": o.elapsed_ms,
                "write": o.write,
            }),
            Value::Null,
        ),
        RunOutcome::Sql(Err(e)) => {
            let err = match e {
                SqlError::InvalidArgs(v) => json!({
                    "kind": "invalid_args", "message": v.to_string(), "fields": v.errors,
                }),
                SqlError::Rejected(m) => json!({ "kind": "rejected", "message": m }),
                SqlError::Config(m) => json!({ "kind": "config", "message": m }),
                SqlError::Connect(m) => json!({ "kind": "connect", "message": m }),
                SqlError::Timeout(_) => json!({ "kind": "timeout", "message": e.to_string() }),
                SqlError::Database(m) => json!({ "kind": "database", "message": m }),
                SqlError::TooManyRows { affected, max, .. } => json!({
                    "kind": "too_many_rows", "message": e.to_string(), "affected_rows": affected, "max": max,
                }),
            };
            (Value::Null, Value::Null, err)
        }
        RunOutcome::Http(Ok(o)) => (
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
        RunOutcome::Http(Err(e)) => {
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
        "kind": tool.kind.type_name(),
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
    #[serde(default)]
    confirm_full_access: bool,
}

async fn import_bundle(
    State(s): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
    ApiJson(req): ApiJson<BundleImportRequest>,
) -> ApiResult<Json<forge_store::BundleReport>> {
    let bundle =
        crate::bundle::parse_bundle(&req.content).map_err(|e| ApiError::Store(e.into()))?;
    for (i, t) in bundle.tools.iter().enumerate() {
        check_shape(&t.input).map_err(|e| match e {
            ApiError::Store(StoreError::Validation(mut v)) => {
                for f in &mut v.errors {
                    f.field = format!("tools[{i}].{}", f.field);
                }
                ApiError::Store(StoreError::Validation(v))
            }
            other => other,
        })?;
    }
    crate::bundle::check_full_access(&bundle, req.confirm_full_access)?;
    crate::bundle::check_sql_tools(&s.store, project_id, &bundle).await?;
    let report = s
        .store
        .import_bundle(
            project_id,
            &bundle.auth_configs,
            &bundle.db_connections,
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
