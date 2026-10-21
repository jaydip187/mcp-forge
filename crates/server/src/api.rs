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
use forge_store::StoreError;
use serde_json::json;
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
            "/projects/{project_id}/tools",
            get(list_tools).post(create_tool),
        )
        .route(
            "/projects/{project_id}/tools/{tool_id}",
            get(get_tool).put(update_tool).delete(delete_tool),
        )
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
    Ok((
        StatusCode::CREATED,
        Json(s.store.create_tool(project_id, &input).await?),
    ))
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
    Ok(Json(s.store.update_tool(project_id, id, &input).await?))
}

async fn delete_tool(
    State(s): State<AppState>,
    ApiPath((project_id, id)): ApiPath<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    s.store.delete_tool(project_id, id).await?;
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
