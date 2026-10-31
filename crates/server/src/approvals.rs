//! Approvals API: calls waiting for a person, across all projects, and the
//! answer from the web UI.

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use forge_core::ApprovalDecision;
use forge_mcp::approvals::PendingApproval;
use forge_store::Access;
use forge_store::StoreError;
use serde::Serialize;
use uuid::Uuid;

use crate::api::{ApiError, ApiPath};
use crate::login::CurrentUser;
use crate::permissions;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/approvals", get(list))
        .route("/approvals/{id}/approve", post(approve))
        .route("/approvals/{id}/deny", post(deny))
}

#[derive(Serialize)]
struct Listed {
    #[serde(flatten)]
    pending: PendingApproval,
    project_name: String,
    project_slug: String,
}

/// Oldest first, only projects the user can write to. Calls of deleted
/// projects are left out (they time out).
async fn list(
    State(s): State<AppState>,
    user: Option<Extension<CurrentUser>>,
) -> Result<Json<Vec<Listed>>, ApiError> {
    let pending = s.exec.approvals.list();
    if pending.is_empty() {
        return Ok(Json(vec![]));
    }
    let user = user.map(|Extension(u)| u);
    let projects = s.store.list_projects().await?;
    let mut out = vec![];
    for p in pending {
        let Some(project) = projects.iter().find(|x| x.id == p.project_id) else {
            continue;
        };
        if permissions::access(&s.store, user.as_ref(), project.id).await? != Some(Access::Write) {
            continue;
        }
        out.push(Listed {
            project_name: project.name.clone(),
            project_slug: project.slug.clone(),
            pending: p,
        });
    }
    Ok(Json(out))
}

/// Answering needs write on the call's project.
async fn ensure_write(s: &AppState, user: Option<&CurrentUser>, id: Uuid) -> Result<(), ApiError> {
    let Some(p) = s.exec.approvals.get(id) else {
        return Err(ApiError::Store(StoreError::NotFound("pending approval")));
    };
    if permissions::access(&s.store, user, p.project_id).await? == Some(Access::Write) {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

fn decider(user: Option<CurrentUser>) -> String {
    match user {
        Some(u) => u.user.username,
        // The admin token was used, or sign-in is off.
        None => "web UI".into(),
    }
}

fn answer(
    s: &AppState,
    id: Uuid,
    decision: ApprovalDecision,
    by: String,
) -> Result<StatusCode, ApiError> {
    if s.exec.approvals.decide(id, decision, Some(by)) {
        Ok(StatusCode::NO_CONTENT)
    } else {
        // Already answered, timed out or cancelled.
        Err(ApiError::Store(StoreError::NotFound("pending approval")))
    }
}

async fn approve(
    State(s): State<AppState>,
    user: Option<Extension<CurrentUser>>,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<StatusCode, ApiError> {
    let user = user.map(|Extension(u)| u);
    ensure_write(&s, user.as_ref(), id).await?;
    answer(&s, id, ApprovalDecision::Approved, decider(user))
}

async fn deny(
    State(s): State<AppState>,
    user: Option<Extension<CurrentUser>>,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<StatusCode, ApiError> {
    let user = user.map(|Extension(u)| u);
    ensure_write(&s, user.as_ref(), id).await?;
    answer(&s, id, ApprovalDecision::Denied, decider(user))
}
