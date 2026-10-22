//! Request guard for the admin surface.
//!
//! - Loopback binds (the default) accept only loopback `Host` headers, which
//!   blocks DNS-rebinding attacks from web pages in the user's browser.
//! - Non-loopback binds require an admin token, sent as
//!   `Authorization: Bearer <token>` on `/api` and `/mcp`.
//!
//! The static UI shell holds no data and is served without a token; it asks
//! for the token and sends it with every API call.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::header::{AUTHORIZATION, HOST, WWW_AUTHENTICATE};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::AppState;

/// Minimum admin token length, so a guessable token can't be used.
pub const MIN_TOKEN_LEN: usize = 16;

#[derive(Clone, Debug)]
pub struct Security {
    /// Required bearer token for `/api` and `/mcp`, if set.
    pub admin_token: Option<Arc<str>>,
    /// Reject requests whose `Host` is not a loopback name.
    pub loopback_hosts_only: bool,
}

impl Default for Security {
    fn default() -> Self {
        Self {
            admin_token: None,
            loopback_hosts_only: true,
        }
    }
}

pub async fn guard(State(state): State<AppState>, req: Request, next: Next) -> Response {
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
    let protected = path.starts_with("/api") || path.starts_with("/mcp");
    if let (true, Some(expected)) = (protected, &sec.admin_token) {
        let given = req
            .headers()
            .get(AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.strip_prefix("Bearer "));
        if !given.is_some_and(|g| constant_time_eq(g.as_bytes(), expected.as_bytes())) {
            let mut resp = error(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "missing or wrong admin token (send `Authorization: Bearer <token>`)",
            );
            resp.headers_mut()
                .insert(WWW_AUTHENTICATE, "Bearer".parse().expect("static header"));
            return resp;
        }
    }
    next.run(req).await
}

fn error(status: StatusCode, code: &str, message: &str) -> Response {
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

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
#[path = "tests/security.rs"]
mod tests;
