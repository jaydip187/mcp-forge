//! Approvals API: calls waiting for a person, across all projects, and the
//! answer from the web UI.

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use forge_core::ApprovalDecision;
use forge_mcp::approvals::PendingApproval;
use forge_store::StoreError;
use serde::Serialize;
use uuid::Uuid;

use crate::api::{ApiError, ApiPath};
use crate::login::CurrentUser;
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

/// Oldest first. Calls of deleted projects are left out (they time out).
async fn list(State(s): State<AppState>) -> Result<Json<Vec<Listed>>, ApiError> {
    let pending = s.exec.approvals.list();
    if pending.is_empty() {
        return Ok(Json(vec![]));
    }
    let projects = s.store.list_projects().await?;
    Ok(Json(
        pending
            .into_iter()
            .filter_map(|p| {
                let project = projects.iter().find(|x| x.id == p.project_id)?;
                Some(Listed {
                    project_name: project.name.clone(),
                    project_slug: project.slug.clone(),
                    pending: p,
                })
            })
            .collect(),
    ))
}

fn decider(user: Option<Extension<CurrentUser>>) -> String {
    match user {
        Some(Extension(u)) => u.user.username,
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
    answer(&s, id, ApprovalDecision::Approved, decider(user))
}

async fn deny(
    State(s): State<AppState>,
    user: Option<Extension<CurrentUser>>,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<StatusCode, ApiError> {
    answer(&s, id, ApprovalDecision::Denied, decider(user))
}
