//! Access keys for a project's `/mcp` endpoint.

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{delete, get};
use axum::{Extension, Json, Router};
use forge_store::{ApiKey, ApiKeyInput};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::api::{ApiError, ApiJson, ApiPath};
use crate::login::CurrentUser;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/projects/{project_id}/api-keys", get(list).post(create))
        .route("/projects/{project_id}/api-keys/{key_id}", delete(remove))
}

async fn list(
    State(s): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
) -> Result<Json<Vec<ApiKey>>, ApiError> {
    Ok(Json(s.store.list_api_keys(project_id).await?))
}

/// The answer is the only time the key itself is shown.
async fn create(
    State(s): State<AppState>,
    user: Option<Extension<CurrentUser>>,
    ApiPath(project_id): ApiPath<Uuid>,
    ApiJson(input): ApiJson<ApiKeyInput>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let by = user.map(|Extension(u)| u.user.username);
    let (key, secret) = s
        .store
        .create_api_key(project_id, &input, by.as_deref())
        .await?;
    tracing::info!(project = %project_id, key = %key.name, "access key created");
    Ok((
        StatusCode::CREATED,
        Json(json!({ "key": key, "secret": secret })),
    ))
}

async fn remove(
    State(s): State<AppState>,
    ApiPath((project_id, key_id)): ApiPath<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    s.store.delete_api_key(project_id, key_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
