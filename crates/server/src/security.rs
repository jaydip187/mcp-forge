//! Request guard for the admin surface.
//!
//! - Loopback binds (the default) accept only loopback `Host` headers, which
//!   blocks DNS-rebinding attacks from web pages in the user's browser.
//! - `/api` needs a signed-in session (see [`crate::login`]) unless
//!   `require_login` is off; state-changing calls made with the session cookie
//!   must carry the [`CSRF_HEADER`].
//! - With an admin token (mandatory on non-loopback binds), `/mcp` requires it
//!   as `Authorization: Bearer <token>`, and `/api` accepts it in place of a
//!   session, for scripts.
//!
//! The static UI shell holds no data and is served to anyone; it shows the
//! sign-in screen and sends the session cookie with every API call.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::header::{AUTHORIZATION, HOST, WWW_AUTHENTICATE};
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::login::{self, CSRF_HEADER};
use crate::permissions;
use crate::AppState;

/// Minimum admin token length, so a guessable token can't be used.
pub const MIN_TOKEN_LEN: usize = 16;

#[derive(Clone, Debug)]
pub struct Security {
    /// Required bearer token for `/api` and `/mcp`, if set.
    pub admin_token: Option<Arc<str>>,
    /// Reject requests whose `Host` is not a loopback name.
    pub loopback_hosts_only: bool,
    /// Require a signed-in session (or the admin token) on `/api`.
    pub require_login: bool,
}

impl Default for Security {
    fn default() -> Self {
        Self {
            admin_token: None,
            loopback_hosts_only: true,
            require_login: true,
        }
    }
}

pub async fn guard(State(state): State<AppState>, mut req: Request, next: Next) -> Response {
    let sec = &state.security;
    let path = req.uri().path();
    if path == "/healthz" {
        return next.run(req).await;
    }
    if sec.loopback_hosts_only {
        let host = req.headers().get(HOST).and_then(|h| h.to_str().ok());
        if !host.is_some_and(is_loopback_host) {
            return error(
                StatusCode::FORBIDDEN,
                "forbidden_host",
                "requests must use a loopback host (localhost, 127.0.0.1 or [::1])",
            );
        }
    }
    let is_api = path.starts_with("/api");
    let is_mcp = path.starts_with("/mcp");
    if !is_api && !is_mcp {
        return next.run(req).await;
    }
    let bearer_ok = sec.admin_token.as_ref().is_some_and(|expected| {
        req.headers()
            .get(AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.strip_prefix("Bearer "))
            .is_some_and(|g| constant_time_eq(g.as_bytes(), expected.as_bytes()))
    });
    if bearer_ok {
        return next.run(req).await;
    }
    if is_mcp {
        return mcp_access(&state, req, next).await;
    }
    if !sec.require_login {
        if sec.admin_token.is_some() {
            return unauthorized_token();
        }
        return next.run(req).await;
    }
    let safe = matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    if !safe && !req.headers().contains_key(CSRF_HEADER) {
        return error(
            StatusCode::FORBIDDEN,
            "csrf",
            "state-changing requests need the `X-Mcp-Forge` header",
        );
    }
    if login::is_public(path) {
        return next.run(req).await;
    }
    match login::authenticate(&state.store, state.login.cookie(), req.headers()).await {
        Ok(Some(user)) => {
            let need = permissions::need(req.method(), path.strip_prefix("/api").unwrap_or(path));
            match permissions::allowed(&state.store, Some(&user), need).await {
                Ok(true) => {
                    req.extensions_mut().insert(user);
                    next.run(req).await
                }
                Ok(false) => error(
                    StatusCode::FORBIDDEN,
                    "forbidden",
                    "your account doesn't have permission for this (ask an admin)",
                ),
                Err(e) => {
                    tracing::error!(error = %e, "permission check failed");
                    error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "internal",
                        "internal error; see server logs",
                    )
                }
            }
        }
        Ok(None) => error(
            StatusCode::UNAUTHORIZED,
            "sign_in_required",
            "sign in first (or send the admin token as `Authorization: Bearer <token>`)",
        ),
        Err(e) => {
            tracing::error!(error = %e, "session lookup failed");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "internal error; see server logs",
            )
        }
    }
}

/// `/mcp/{slug}` without the admin token: an access key for that project
/// lets the request in (with its tool list attached); without one, the
/// project must not require a key and the server must not require the
/// admin token.
async fn mcp_access(state: &AppState, mut req: Request, next: Next) -> Response {
    let slug = req
        .uri()
        .path()
        .strip_prefix("/mcp/")
        .unwrap_or("")
        .split('/')
        .next()
        .unwrap_or("")
        .to_owned();
    let bearer = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .map(str::to_owned);
    let project = match state.store.get_project_by_slug(&slug).await {
        Ok(p) => Some(p),
        Err(forge_store::StoreError::NotFound(_)) => None,
        Err(e) => {
            tracing::error!(error = %e, "resolving MCP project");
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "internal error; see server logs",
            );
        }
    };
    let www = format!(
        "Bearer resource_metadata=\"{}\"",
        crate::authz::resource_metadata_url(req.headers(), &slug)
    );
    let deny = |message: &str| {
        let mut resp = error(StatusCode::UNAUTHORIZED, "unauthorized", message);
        if let Ok(v) = www.parse() {
            resp.headers_mut().insert(WWW_AUTHENTICATE, v);
        }
        resp
    };
    let internal = || {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "internal error; see server logs",
        )
    };
    // Signed in through mcp-forge's OAuth server.
    if let Some(token) = bearer
        .as_deref()
        .filter(|b| b.starts_with(forge_store::ACCESS_PREFIX))
    {
        return match (state.store.find_oauth_access(token).await, project) {
            (Ok(Some(g)), Some(p)) if g.project_id == p.id => {
                req.extensions_mut().insert(forge_mcp::KeyScope {
                    name: format!("{} (sign-in)", g.client_name),
                    tools: None,
                });
                next.run(req).await
            }
            (Err(e), _) => {
                tracing::error!(error = %e, "looking up OAuth token");
                internal()
            }
            _ => deny("the access token is unknown, expired, revoked or for another project"),
        };
    }
    if let Some(secret) = bearer
        .as_deref()
        .filter(|b| b.starts_with(forge_store::KEY_PREFIX))
    {
        let key = match state.store.find_api_key(secret).await {
            Ok(k) => k,
            Err(e) => {
                tracing::error!(error = %e, "looking up access key");
                return internal();
            }
        };
        return match (key, project) {
            (Some(k), Some(p)) if k.project_id == p.id && !k.expired(chrono::Utc::now()) => {
                req.extensions_mut().insert(forge_mcp::KeyScope {
                    name: k.name,
                    tools: k.tools,
                });
                next.run(req).await
            }
            (Some(k), Some(p)) if k.project_id == p.id => deny("this access key has expired"),
            _ => deny("unknown access key, or a key for another project"),
        };
    }
    if state.security.admin_token.is_some() {
        return deny(
            "sign in, or send an access key or the admin token as `Authorization: Bearer …`",
        );
    }
    if project.is_some_and(|p| p.settings.require_key) {
        return deny(
            "this project needs sign-in or an access key (`Authorization: Bearer mcpf_…`)",
        );
    }
    // Unknown slugs fall through to the handler's 404.
    next.run(req).await
}

fn unauthorized_token() -> Response {
    let mut resp = error(
        StatusCode::UNAUTHORIZED,
        "unauthorized",
        "missing or wrong admin token (send `Authorization: Bearer <token>`)",
    );
    resp.headers_mut()
        .insert(WWW_AUTHENTICATE, "Bearer".parse().expect("static header"));
    resp
}

pub(crate) fn error(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        Json(json!({ "error": { "code": code, "message": message } })),
    )
        .into_response()
}

/// `localhost`, `127.x.x.x` or `::1`, with or without a port.
pub fn is_loopback_host(host: &str) -> bool {
    let name = if let Some(rest) = host.strip_prefix('[') {
        rest.split(']').next().unwrap_or("")
    } else {
        host.rsplit_once(':').map_or(host, |(h, _)| h)
    };
    name.eq_ignore_ascii_case("localhost")
        || name
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
#[path = "../tests/unit/security.rs"]
mod tests;
