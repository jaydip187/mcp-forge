//! Who may do what on `/api`: admins everything; members only their
//! projects, read (GET) or write (everything else). Decided from the path
//! and method in one place, so a new route is admin-only until listed here.

use axum::http::Method;
use forge_store::{Access, Role, Store, StoreError};
use uuid::Uuid;

use crate::login::CurrentUser;

/// What a request needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    /// Any signed-in user (handlers filter by project themselves).
    SignedIn,
    Read(Uuid),
    Write(Uuid),
    Admin,
}

/// The requirement for `method path` (`path` without the `/api` prefix).
pub fn need(method: &Method, path: &str) -> Need {
    let read = matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS);
    let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
    match parts.as_slice() {
        ["auth", ..] | ["info"] => Need::SignedIn,
        // Filtered by the handler: only projects the user can see.
        ["projects"] if read => Need::SignedIn,
        ["projects"] => Need::Admin,
        // Deleting a project is admin-only.
        ["projects", _] if *method == Method::DELETE => Need::Admin,
        // Filling a prompt in changes nothing.
        ["projects", id, "prompts", _, "render"] if *method == Method::POST => {
            Uuid::parse_str(id).map_or(Need::Admin, Need::Read)
        }
        ["projects", id, ..] => match Uuid::parse_str(id) {
            Ok(pid) if read => Need::Read(pid),
            Ok(pid) => Need::Write(pid),
            Err(_) => Need::Admin,
        },
        // Each item belongs to a project; the handler checks write on it.
        ["approvals", ..] | ["oauth", "requests", ..] => Need::SignedIn,
        _ => Need::Admin,
    }
}

/// A user's permission on a project: admins and unauthenticated requests
/// that got past the guard (admin token, sign-in off) have write.
pub async fn access(
    store: &Store,
    user: Option<&CurrentUser>,
    project_id: Uuid,
) -> Result<Option<Access>, StoreError> {
    match user {
        None => Ok(Some(Access::Write)),
        Some(u) if u.user.role == Role::Admin => Ok(Some(Access::Write)),
        Some(u) => store.project_access(u.user.id, project_id).await,
    }
}

pub fn is_admin(user: Option<&CurrentUser>) -> bool {
    user.is_none_or(|u| u.user.role == Role::Admin)
}

/// Whether `user` meets `need`. `Err` is a store problem.
pub async fn allowed(
    store: &Store,
    user: Option<&CurrentUser>,
    need: Need,
) -> Result<bool, StoreError> {
    Ok(match need {
        Need::SignedIn => true,
        Need::Admin => is_admin(user),
        Need::Read(pid) => access(store, user, pid).await?.is_some(),
        Need::Write(pid) => access(store, user, pid).await? == Some(Access::Write),
    })
}

#[cfg(test)]
#[path = "../tests/unit/permissions.rs"]
mod tests;
