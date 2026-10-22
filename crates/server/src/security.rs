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
    if is_mcp || !sec.require_login {
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
    match login::authenticate(&state.store, req.headers()).await {
        Ok(Some(user)) => {
            req.extensions_mut().insert(user);
            next.run(req).await
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
#[path = "tests/security.rs"]
mod tests;
