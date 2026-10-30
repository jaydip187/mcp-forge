//! OAuth2 authorization code flow for upstream APIs ("Connect" in the UI).
//!
//! `POST …/oauth/start` remembers a random `state` and PKCE verifier and
//! returns the provider's authorize URL; the provider sends the browser back
//! to `/oauth/callback`, which trades the code for tokens and stores them.
//! The callback can't rely on the session cookie (it's `SameSite=Strict` and
//! the provider's redirect is cross-site), so the single-use, short-lived
//! `state` is what ties it to the signed-in user who started it.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::{Query, State};
use axum::http::header::{HOST, LOCATION};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use chrono::Utc;
use forge_core::AuthKind;
use forge_exec_http::oauth::{authorize_url, pkce_pair, random_token};
use forge_store::{OAuthStatus, OAuthToken, StoreError};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::api::{ApiError, ApiPath};
use crate::login::CurrentUser;
use crate::AppState;

/// How long a started Connect flow stays valid.
const FLOW_TTL: Duration = Duration::from_secs(10 * 60);
/// Bound on started-but-unfinished flows.
const MAX_FLOWS: usize = 1_000;
pub const CALLBACK_PATH: &str = "/oauth/callback";

struct Flow {
    project_id: Uuid,
    auth_id: Uuid,
    verifier: String,
    redirect_uri: String,
    started: Instant,
    by: Option<String>,
}

/// Started Connect flows by `state`.
#[derive(Default)]
pub struct OAuthFlows {
    flows: Mutex<HashMap<String, Flow>>,
}

impl OAuthFlows {
    fn insert(&self, state: String, flow: Flow) {
        let mut flows = self.flows.lock().expect("oauth flows poisoned");
        flows.retain(|_, f| f.started.elapsed() < FLOW_TTL);
        if flows.len() >= MAX_FLOWS {
            if let Some(oldest) = flows
                .iter()
                .min_by_key(|(_, f)| f.started)
                .map(|(k, _)| k.clone())
            {
                flows.remove(&oldest);
            }
        }
        flows.insert(state, flow);
    }

    /// Takes the flow for `state` (single use) if it hasn't expired.
    fn take(&self, state: &str) -> Option<Flow> {
        let flow = self
            .flows
            .lock()
            .expect("oauth flows poisoned")
            .remove(state)?;
        (flow.started.elapsed() < FLOW_TTL).then_some(flow)
    }
}

pub fn api_router() -> Router<AppState> {
    Router::new()
        .route("/projects/{project_id}/oauth", get(status))
        .route(
            "/projects/{project_id}/auth-configs/{auth_id}/oauth/start",
            post(start),
        )
        .route(
            "/projects/{project_id}/auth-configs/{auth_id}/oauth",
            axum::routing::delete(disconnect),
        )
}

pub fn callback_router() -> Router<AppState> {
    Router::new().route(CALLBACK_PATH, get(callback))
}

/// The callback URL as the browser reaches this server: register it at the
/// provider.
pub fn redirect_uri(headers: &HeaderMap) -> Option<String> {
    let host = headers.get(HOST)?.to_str().ok()?;
    let https = headers
        .get("x-forwarded-proto")
        .is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"https"));
    Some(format!(
        "{}://{host}{CALLBACK_PATH}",
        if https { "https" } else { "http" }
    ))
}

async fn status(
    State(s): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
) -> Result<Json<Vec<OAuthStatus>>, ApiError> {
    Ok(Json(s.store.list_oauth_status(project_id).await?))
}

async fn start(
    State(s): State<AppState>,
    user: Option<Extension<CurrentUser>>,
    headers: HeaderMap,
    ApiPath((project_id, auth_id)): ApiPath<(Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    let cfg = s.store.get_auth_config(project_id, auth_id).await?;
    if !matches!(cfg.kind, AuthKind::OAuth2AuthorizationCode { .. }) {
        return Err(ApiError::BadRequest(format!(
            "auth config `{}` doesn't use the authorization code flow",
            cfg.name
        )));
    }
    let redirect = redirect_uri(&headers)
        .ok_or_else(|| ApiError::BadRequest("the request has no Host header".into()))?;
    let state = random_token(32);
    let (verifier, challenge) = pkce_pair();
    let url = authorize_url(&cfg, &redirect, &state, &challenge)
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    s.oauth_flows.insert(
        state,
        Flow {
            project_id,
            auth_id,
            verifier,
            redirect_uri: redirect.clone(),
            started: Instant::now(),
            by: user.map(|Extension(u)| u.user.username),
        },
    );
    Ok(Json(
        json!({ "authorize_url": url.as_str(), "redirect_uri": redirect }),
    ))
}

async fn disconnect(
    State(s): State<AppState>,
    ApiPath((project_id, auth_id)): ApiPath<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    s.store.get_auth_config(project_id, auth_id).await?;
    s.store.delete_oauth_token(auth_id).await?;
    s.exec.http.tokens().forget(auth_id);
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct CallbackQuery {
    state: Option<String>,
    code: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// A small standalone page for callback problems (the SPA isn't loaded here).
fn problem(status: StatusCode, message: &str, back: &str) -> Response {
    let page = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>Couldn't connect · mcp-forge</title>\
         <style>body{{font:15px/1.5 system-ui,sans-serif;max-width:36rem;margin:15vh auto;padding:0 1.5rem;color:#1f2933}}\
         h1{{font-size:1.25rem}}a{{color:#0b7374}}</style>\
         <h1>Couldn't connect</h1><p>{}</p><p><a href=\"{}\">Back to mcp-forge</a></p>",
        escape(message),
        escape(back)
    );
    (status, Html(page)).into_response()
}

async fn callback(State(s): State<AppState>, Query(q): Query<CallbackQuery>) -> Response {
    let Some(flow) = q.state.as_deref().and_then(|st| s.oauth_flows.take(st)) else {
        return problem(
            StatusCode::BAD_REQUEST,
            "This sign-in link is unknown, already used or older than 10 minutes. Start again with Connect.",
            "/",
        );
    };
    let back = format!("/#/p/{}/auth", flow.project_id);
    if let Some(err) = q.error {
        let detail = q
            .error_description
            .map(|d| format!(" ({d})"))
            .unwrap_or_default();
        return problem(
            StatusCode::BAD_REQUEST,
            &format!("The provider didn't grant access: {err}{detail}."),
            &back,
        );
    }
    let Some(code) = q.code.filter(|c| !c.is_empty()) else {
        return problem(StatusCode::BAD_REQUEST, "The provider sent no code.", &back);
    };
    let cfg = match s.store.get_auth_config(flow.project_id, flow.auth_id).await {
        Ok(c) => c,
        Err(StoreError::NotFound(_)) => {
            return problem(
                StatusCode::NOT_FOUND,
                "The auth config was deleted meanwhile.",
                &back,
            )
        }
        Err(e) => {
            tracing::error!(error = %e, "oauth callback");
            return problem(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal error; see the server log.",
                &back,
            );
        }
    };
    let tokens = match s
        .exec
        .http
        .exchange_code(&cfg, &code, &flow.verifier, &flow.redirect_uri)
        .await
    {
        Ok(t) => t,
        Err(e) => return problem(StatusCode::BAD_GATEWAY, &e.to_string(), &back),
    };
    let token = OAuthToken {
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        expires_at: tokens.expires_at,
        scope: tokens.scope,
        connected_at: Utc::now(),
        connected_by: flow.by,
    };
    if let Err(e) = s.store.put_oauth_token(cfg.id, &token).await {
        tracing::error!(error = %e, "storing oauth tokens");
        return problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal error; see the server log.",
            &back,
        );
    }
    s.exec.http.tokens().forget(cfg.id);
    tracing::info!(auth = %cfg.name, refresh = token.refresh_token.is_some(), "OAuth config connected");
    (StatusCode::SEE_OTHER, [(LOCATION, back)]).into_response()
}
