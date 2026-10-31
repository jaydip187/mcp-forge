//! Accounts and their per-project access, for admins; plus who can see a
//! project (its People card).

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, put};
use axum::{Extension, Json, Router};
use forge_core::ValidationErrors;
use forge_store::{Access, ProjectPerson, Role, StoreError, User};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::{ApiError, ApiJson, ApiPath};
use crate::login::{check_password, check_username, hash_password, CurrentUser};
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/users", get(list).post(create))
        .route("/users/{user_id}", put(update).delete(remove))
        .route("/users/{user_id}/password", put(reset_password))
        .route("/users/{user_id}/access/{project_id}", put(set_access))
        .route("/projects/{project_id}/people", get(people))
}

#[derive(Serialize)]
struct UserView {
    #[serde(flatten)]
    user: User,
    /// Members' projects; empty for admins (they see everything).
    access: Vec<ProjectAccess>,
}

#[derive(Serialize)]
struct ProjectAccess {
    project_id: Uuid,
    access: Access,
}

fn invalid(field: &str, msg: String) -> ApiError {
    ApiError::Store(StoreError::Validation(ValidationErrors::single(field, msg)))
}

async fn view(s: &AppState, user: User) -> Result<UserView, ApiError> {
    let access = if user.role == Role::Member {
        s.store
            .user_access(user.id)
            .await?
            .into_iter()
            .map(|(project_id, access)| ProjectAccess { project_id, access })
            .collect()
    } else {
        vec![]
    };
    Ok(UserView { user, access })
}

async fn list(State(s): State<AppState>) -> Result<Json<Vec<UserView>>, ApiError> {
    let mut out = vec![];
    for u in s.store.list_users().await? {
        out.push(view(&s, u).await?);
    }
    Ok(Json(out))
}

#[derive(Deserialize)]
struct NewUser {
    username: String,
    password: String,
    role: Role,
}

async fn create(
    State(s): State<AppState>,
    ApiJson(req): ApiJson<NewUser>,
) -> Result<(StatusCode, Json<UserView>), ApiError> {
    let username = req.username.trim().to_owned();
    check_username(&username).map_err(|m| invalid("username", m))?;
    check_password(&req.password).map_err(|m| invalid("password", m))?;
    let password = req.password;
    let phc = tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .expect("hashing task");
    let user = s.store.create_user_as(&username, &phc, req.role).await?;
    tracing::info!(username = %user.username, role = user.role.as_str(), "account created");
    Ok((StatusCode::CREATED, Json(view(&s, user).await?)))
}

#[derive(Deserialize)]
struct RoleUpdate {
    role: Role,
}

async fn update(
    State(s): State<AppState>,
    ApiPath(user_id): ApiPath<Uuid>,
    ApiJson(req): ApiJson<RoleUpdate>,
) -> Result<Json<UserView>, ApiError> {
    let user = s.store.set_user_role(user_id, req.role).await?;
    Ok(Json(view(&s, user).await?))
}

#[derive(Deserialize)]
struct PasswordReset {
    password: String,
}

/// Sets a new password and ends the user's sessions (all of them, unless
/// it's your own account: then this session stays).
async fn reset_password(
    State(s): State<AppState>,
    current: Option<Extension<CurrentUser>>,
    ApiPath(user_id): ApiPath<Uuid>,
    ApiJson(req): ApiJson<PasswordReset>,
) -> Result<StatusCode, ApiError> {
    check_password(&req.password).map_err(|m| invalid("password", m))?;
    let password = req.password;
    let phc = tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .expect("hashing task");
    let keep = current
        .as_ref()
        .filter(|Extension(c)| c.user.id == user_id)
        .map(|Extension(c)| c.token_hash.clone());
    s.store.set_password(user_id, &phc, keep.as_deref()).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove(
    State(s): State<AppState>,
    current: Option<Extension<CurrentUser>>,
    ApiPath(user_id): ApiPath<Uuid>,
) -> Result<StatusCode, ApiError> {
    if current.is_some_and(|Extension(c)| c.user.id == user_id) {
        return Err(ApiError::Store(StoreError::Conflict(
            "you can't remove your own account here; another admin can".into(),
        )));
    }
    let user = s.store.get_user(user_id).await?;
    s.store.delete_user(&user.username).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct AccessUpdate {
    /// `null` removes the user's access to the project.
    access: Option<Access>,
}

async fn set_access(
    State(s): State<AppState>,
    ApiPath((user_id, project_id)): ApiPath<(Uuid, Uuid)>,
    ApiJson(req): ApiJson<AccessUpdate>,
) -> Result<StatusCode, ApiError> {
    s.store
        .set_project_access(user_id, project_id, req.access)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn people(
    State(s): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
) -> Result<Json<Vec<ProjectPerson>>, ApiError> {
    Ok(Json(s.store.project_people(project_id).await?))
}
