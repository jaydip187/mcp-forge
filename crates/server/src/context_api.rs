//! REST API for MCP resources and prompts, plus a view of exactly what
//! agents see (written resources, database schemas, shared specs).

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use forge_core::{PromptDef, PromptInput, ResourceDef, ResourceInput};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::api::{ApiError, ApiJson, ApiPath};
use crate::AppState;

type ApiResult<T> = Result<T, ApiError>;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/projects/{project_id}/resources",
            get(list_resources).post(create_resource),
        )
        .route(
            "/projects/{project_id}/resources/{resource_id}",
            get(get_resource)
                .put(update_resource)
                .delete(delete_resource),
        )
        .route(
            "/projects/{project_id}/prompts",
            get(list_prompts).post(create_prompt),
        )
        .route(
            "/projects/{project_id}/prompts/{prompt_id}",
            get(get_prompt).put(update_prompt).delete(delete_prompt),
        )
        .route(
            "/projects/{project_id}/prompts/{prompt_id}/render",
            axum::routing::post(render_prompt),
        )
        .route("/projects/{project_id}/mcp-resources", get(mcp_resources))
        .route(
            "/projects/{project_id}/mcp-resources/read",
            get(read_mcp_resource),
        )
}

async fn list_resources(
    State(s): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
) -> ApiResult<Json<Vec<ResourceDef>>> {
    Ok(Json(s.store.list_resources(project_id).await?))
}

async fn get_resource(
    State(s): State<AppState>,
    ApiPath((project_id, id)): ApiPath<(Uuid, Uuid)>,
) -> ApiResult<Json<ResourceDef>> {
    Ok(Json(s.store.get_resource(project_id, id).await?))
}

async fn create_resource(
    State(s): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
    ApiJson(input): ApiJson<ResourceInput>,
) -> ApiResult<(StatusCode, Json<ResourceDef>)> {
    let r = s.store.create_resource(project_id, &input).await?;
    s.changes.notify(project_id);
    Ok((StatusCode::CREATED, Json(r)))
}

async fn update_resource(
    State(s): State<AppState>,
    ApiPath((project_id, id)): ApiPath<(Uuid, Uuid)>,
    ApiJson(input): ApiJson<ResourceInput>,
) -> ApiResult<Json<ResourceDef>> {
    let r = s.store.update_resource(project_id, id, &input).await?;
    s.changes.notify(project_id);
    Ok(Json(r))
}

async fn delete_resource(
    State(s): State<AppState>,
    ApiPath((project_id, id)): ApiPath<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    s.store.delete_resource(project_id, id).await?;
    s.changes.notify(project_id);
    Ok(StatusCode::NO_CONTENT)
}

async fn list_prompts(
    State(s): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
) -> ApiResult<Json<Vec<PromptDef>>> {
    Ok(Json(s.store.list_prompts(project_id).await?))
}

async fn get_prompt(
    State(s): State<AppState>,
    ApiPath((project_id, id)): ApiPath<(Uuid, Uuid)>,
) -> ApiResult<Json<PromptDef>> {
    Ok(Json(s.store.get_prompt(project_id, id).await?))
}

async fn create_prompt(
    State(s): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
    ApiJson(input): ApiJson<PromptInput>,
) -> ApiResult<(StatusCode, Json<PromptDef>)> {
    let p = s.store.create_prompt(project_id, &input).await?;
    s.changes.notify(project_id);
    Ok((StatusCode::CREATED, Json(p)))
}

async fn update_prompt(
    State(s): State<AppState>,
    ApiPath((project_id, id)): ApiPath<(Uuid, Uuid)>,
    ApiJson(input): ApiJson<PromptInput>,
) -> ApiResult<Json<PromptDef>> {
    let p = s.store.update_prompt(project_id, id, &input).await?;
    s.changes.notify(project_id);
    Ok(Json(p))
}

async fn delete_prompt(
    State(s): State<AppState>,
    ApiPath((project_id, id)): ApiPath<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    s.store.delete_prompt(project_id, id).await?;
    s.changes.notify(project_id);
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct RenderRequest {
    #[serde(default)]
    arguments: Map<String, Value>,
}

/// The prompt as an agent would get it, for the UI preview.
async fn render_prompt(
    State(s): State<AppState>,
    ApiPath((project_id, id)): ApiPath<(Uuid, Uuid)>,
    ApiJson(req): ApiJson<RenderRequest>,
) -> ApiResult<Json<Value>> {
    let p = s.store.get_prompt(project_id, id).await?;
    Ok(Json(match p.render(&req.arguments) {
        Ok(text) => json!({ "text": text }),
        Err(e) => json!({ "error": e }),
    }))
}

/// Every resource agents can list, as MCP shows them.
async fn mcp_resources(
    State(s): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
) -> ApiResult<Json<Value>> {
    let settings = s.store.get_project(project_id).await?.settings;
    let items = forge_mcp::context::list_resources(&s.store, project_id, &settings).await?;
    Ok(Json(serde_json::to_value(items).unwrap_or(Value::Null)))
}

#[derive(Deserialize)]
struct ReadQuery {
    uri: String,
}

/// One resource's content, as an agent would read it.
async fn read_mcp_resource(
    State(s): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
    Query(q): Query<ReadQuery>,
) -> ApiResult<Json<Value>> {
    let settings = s.store.get_project(project_id).await?.settings;
    match forge_mcp::context::read_resource(&s.store, &s.exec, project_id, &settings, &q.uri)
        .await?
    {
        forge_mcp::context::Read::Found { text, mime } => Ok(Json(
            json!({ "uri": q.uri, "mime_type": mime, "text": text }),
        )),
        forge_mcp::context::Read::NotFound => Err(ApiError::Store(
            forge_store::StoreError::NotFound("resource"),
        )),
    }
}
