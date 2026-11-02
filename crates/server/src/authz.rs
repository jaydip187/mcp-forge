//! mcp-forge as the OAuth 2.1 authorization server for its own `/mcp`
//! endpoints (MCP authorization spec), so clients such as claude.ai custom
//! connectors can connect by signing in instead of pasting a key.
//!
//! - Discovery: `/.well-known/oauth-protected-resource/mcp/{slug}` (RFC 9728)
//!   and `/.well-known/oauth-authorization-server` (RFC 8414).
//! - Clients register themselves at `/oauth/register` (RFC 7591).
//! - `/oauth/authorize` checks the request, then hands the browser to the
//!   web UI's consent page (`#/oauth/consent/{id}`), where a signed-in user
//!   approves. The session cookie is `SameSite=Strict`, so it isn't sent on
//!   the cross-site arrival, but the consent page's own `/api` calls carry it.
//! - `/oauth/token`: authorization code + PKCE (S256 only) and refresh
//!   (rotating). `/oauth/revoke` (RFC 7009).
//! - Each grant covers one project (the `resource` the client asked for).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration as StdDuration, Instant};

use axum::extract::{Form, Query, State};
use axum::http::header::{AUTHORIZATION, CACHE_CONTROL, HOST, LOCATION, WWW_AUTHENTICATE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Extension, Json, Router};
use chrono::Duration;
use forge_exec_http::oauth::{pkce_challenge, random_token};
use forge_store::{IssuedTokens, OAuthGrant, StoreError};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::api::{ApiError, ApiPath};
use crate::login::CurrentUser;
use crate::security::constant_time_eq;
use crate::AppState;

fn access_ttl() -> Duration {
    Duration::hours(1)
}

fn refresh_ttl() -> Duration {
    Duration::days(30)
}

/// How long a consent page stays valid, and an issued code.
const REQUEST_TTL: StdDuration = StdDuration::from_secs(10 * 60);
const CODE_TTL: StdDuration = StdDuration::from_secs(5 * 60);
const MAX_PENDING: usize = 1_000;
const MAX_REDIRECT_URIS: usize = 10;

/// An authorize request waiting for someone to approve it.
#[derive(Clone)]
struct PendingRequest {
    client_id: String,
    client_name: String,
    redirect_uri: String,
    state: Option<String>,
    challenge: String,
    project_id: Uuid,
    started: Instant,
}

/// A code issued after approval, waiting for the token request.
struct IssuedCode {
    client_id: String,
    redirect_uri: String,
    challenge: String,
    project_id: Uuid,
    approved_by: Option<String>,
    issued: Instant,
}

/// Consent requests and codes, in memory (a restart just means signing in again).
#[derive(Default)]
pub struct Authz {
    requests: Mutex<HashMap<String, PendingRequest>>,
    codes: Mutex<HashMap<String, IssuedCode>>,
}

impl Authz {
    fn add_request(&self, id: String, r: PendingRequest) {
        let mut m = self.requests.lock().expect("authz requests poisoned");
        m.retain(|_, r| r.started.elapsed() < REQUEST_TTL);
        if m.len() < MAX_PENDING {
            m.insert(id, r);
        }
    }

    fn request(&self, id: &str) -> Option<PendingRequest> {
        let m = self.requests.lock().expect("authz requests poisoned");
        m.get(id)
            .filter(|r| r.started.elapsed() < REQUEST_TTL)
            .cloned()
    }

    fn take_request(&self, id: &str) -> Option<PendingRequest> {
        let r = self
            .requests
            .lock()
            .expect("authz requests poisoned")
            .remove(id)?;
        (r.started.elapsed() < REQUEST_TTL).then_some(r)
    }

    fn add_code(&self, code: String, c: IssuedCode) {
        let mut m = self.codes.lock().expect("authz codes poisoned");
        m.retain(|_, c| c.issued.elapsed() < CODE_TTL);
        m.insert(code, c);
    }

    fn take_code(&self, code: &str) -> Option<IssuedCode> {
        let c = self
            .codes
            .lock()
            .expect("authz codes poisoned")
            .remove(code)?;
        (c.issued.elapsed() < CODE_TTL).then_some(c)
    }
}

/// `scheme://host` as the client reached this server.
pub fn origin(headers: &HeaderMap) -> String {
    let host = headers
        .get(HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost");
    let https = headers
        .get("x-forwarded-proto")
        .is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"https"));
    format!("{}://{host}", if https { "https" } else { "http" })
}

/// Where a client finds out how to sign in for `slug` (sent in 401s).
pub fn resource_metadata_url(headers: &HeaderMap, slug: &str) -> String {
    format!(
        "{}/.well-known/oauth-protected-resource/mcp/{slug}",
        origin(headers)
    )
}

/// Public endpoints (no cookies, so any origin may call them).
pub fn public_router() -> Router<AppState> {
    Router::new()
        .route(
            "/.well-known/oauth-protected-resource/mcp/{slug}",
            get(protected_resource),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(authorization_server),
        )
        .route("/oauth/register", post(register))
        .route("/oauth/token", post(token))
        .route("/oauth/revoke", post(revoke))
        .layer(tower_http::cors::CorsLayer::permissive())
        // Browser navigation, not fetched cross-origin.
        .route("/oauth/authorize", get(authorize))
}

/// Consent and connected apps, for the signed-in web UI.
pub fn api_router() -> Router<AppState> {
    Router::new()
        .route("/oauth/requests/{id}", get(consent_info))
        .route("/oauth/requests/{id}/approve", post(approve))
        .route("/oauth/requests/{id}/deny", post(deny))
        .route("/projects/{project_id}/connected-apps", get(list_grants))
        .route(
            "/projects/{project_id}/connected-apps/{grant_id}",
            delete(delete_grant),
        )
}

// ---- discovery ---------------------------------------------------------------

async fn protected_resource(
    State(s): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(slug): axum::extract::Path<String>,
) -> Response {
    match s.store.get_project_by_slug(&slug).await {
        Ok(_) => {}
        Err(StoreError::NotFound(_)) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "protected resource metadata");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }
    let o = origin(&headers);
    Json(json!({
        "resource": format!("{o}/mcp/{slug}"),
        "authorization_servers": [o],
        "bearer_methods_supported": ["header"],
        "scopes_supported": ["mcp"],
        "resource_name": format!("mcp-forge project {slug}"),
    }))
    .into_response()
}

async fn authorization_server(headers: HeaderMap) -> Json<Value> {
    let o = origin(&headers);
    Json(json!({
        "issuer": o,
        "authorization_endpoint": format!("{o}/oauth/authorize"),
        "token_endpoint": format!("{o}/oauth/token"),
        "registration_endpoint": format!("{o}/oauth/register"),
        "revocation_endpoint": format!("{o}/oauth/revoke"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none", "client_secret_post", "client_secret_basic"],
        "revocation_endpoint_auth_methods_supported": ["none", "client_secret_post", "client_secret_basic"],
        "scopes_supported": ["mcp"],
        "authorization_response_iss_parameter_supported": true,
    }))
}

// ---- registration --------------------------------------------------------------

#[derive(Deserialize)]
struct RegisterRequest {
    #[serde(default)]
    redirect_uris: Vec<String>,
    client_name: Option<String>,
    token_endpoint_auth_method: Option<String>,
    #[serde(default)]
    grant_types: Option<Vec<String>>,
}

fn oauth_error(status: StatusCode, error: &str, description: &str) -> Response {
    let mut resp = (
        status,
        Json(json!({ "error": error, "error_description": description })),
    )
        .into_response();
    resp.headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp
}

/// https anywhere; http only to loopback (native apps); custom schemes for
/// native apps, except ones that run code or read files.
fn redirect_uri_ok(raw: &str) -> bool {
    let Ok(u) = url::Url::parse(raw) else {
        return false;
    };
    if u.fragment().is_some() {
        return false;
    }
    match u.scheme() {
        "https" => u.has_host(),
        "http" => matches!(u.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")),
        "javascript" | "data" | "file" | "vbscript" | "blob" | "about" => false,
        _ => true,
    }
}

async fn register(State(s): State<AppState>, body: axum::body::Bytes) -> Response {
    let Ok(req) = serde_json::from_slice::<RegisterRequest>(&body) else {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_client_metadata",
            "expected a JSON object",
        );
    };
    if req.redirect_uris.is_empty() || req.redirect_uris.len() > MAX_REDIRECT_URIS {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_redirect_uri",
            "give 1 to 10 redirect_uris",
        );
    }
    if let Some(bad) = req
        .redirect_uris
        .iter()
        .find(|u| !redirect_uri_ok(u) || u.len() > 2000)
    {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_redirect_uri",
            &format!("redirect URI not allowed: {bad}"),
        );
    }
    let method = req
        .token_endpoint_auth_method
        .unwrap_or_else(|| "client_secret_basic".into());
    if !matches!(
        method.as_str(),
        "none" | "client_secret_post" | "client_secret_basic"
    ) {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_client_metadata",
            "token_endpoint_auth_method must be none, client_secret_post or client_secret_basic",
        );
    }
    if let Some(g) = &req.grant_types {
        if g.iter()
            .any(|t| t != "authorization_code" && t != "refresh_token")
        {
            return oauth_error(
                StatusCode::BAD_REQUEST,
                "invalid_client_metadata",
                "only authorization_code and refresh_token grants are supported",
            );
        }
    }
    let name: String = req
        .client_name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or("Unnamed client")
        .chars()
        .take(100)
        .collect();
    match s
        .store
        .register_oauth_client(&name, &req.redirect_uris, &method)
        .await
    {
        Ok((client, secret)) => {
            tracing::info!(client = %client.name, "OAuth client registered");
            let mut body = json!({
                "client_id": client.client_id,
                "client_id_issued_at": client.created_at.timestamp(),
                "client_name": client.name,
                "redirect_uris": client.redirect_uris,
                "token_endpoint_auth_method": client.auth_method,
                "grant_types": ["authorization_code", "refresh_token"],
                "response_types": ["code"],
            });
            if let Some(secret) = secret {
                body["client_secret"] = json!(secret);
                body["client_secret_expires_at"] = json!(0);
            }
            (StatusCode::CREATED, Json(body)).into_response()
        }
        Err(StoreError::Conflict(m)) => {
            oauth_error(StatusCode::BAD_REQUEST, "invalid_client_metadata", &m)
        }
        Err(e) => {
            tracing::error!(error = %e, "registering OAuth client");
            oauth_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                "internal error",
            )
        }
    }
}

// ---- authorize ---------------------------------------------------------------

#[derive(Deserialize)]
struct AuthorizeQuery {
    response_type: Option<String>,
    client_id: Option<String>,
    redirect_uri: Option<String>,
    state: Option<String>,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
    resource: Option<String>,
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// For requests we can't send back to the client (unknown client or redirect URI).
fn problem_page(message: &str) -> Response {
    let page = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>Can't connect · mcp-forge</title>\
         <style>body{{font:15px/1.5 system-ui,sans-serif;max-width:36rem;margin:15vh auto;padding:0 1.5rem;color:#1f2933}}\
         h1{{font-size:1.25rem}}</style><h1>This app can't connect</h1><p>{}</p>",
        escape(message)
    );
    (StatusCode::BAD_REQUEST, Html(page)).into_response()
}

/// Sends an OAuth error back to the client's redirect URI.
fn redirect_error(redirect_uri: &str, error: &str, state: Option<&str>, iss: &str) -> Response {
    let mut u = match url::Url::parse(redirect_uri) {
        Ok(u) => u,
        Err(_) => return problem_page("The app's redirect URI is invalid."),
    };
    {
        let mut q = u.query_pairs_mut();
        q.append_pair("error", error);
        if let Some(st) = state {
            q.append_pair("state", st);
        }
        q.append_pair("iss", iss);
    }
    (StatusCode::SEE_OTHER, [(LOCATION, u.to_string())]).into_response()
}

/// The project a `resource` URL names on this server (`{origin}/mcp/{slug}`).
fn resource_slug<'a>(resource: &'a str, origin: &str) -> Option<&'a str> {
    let rest = resource.strip_prefix(origin)?.strip_prefix("/mcp/")?;
    let slug = rest.trim_end_matches('/');
    (!slug.is_empty() && !slug.contains(['/', '?', '#'])).then_some(slug)
}

async fn authorize(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<AuthorizeQuery>,
) -> Response {
    let iss = origin(&headers);
    let Some(client_id) = q.client_id.as_deref() else {
        return problem_page("The request names no client.");
    };
    let client = match s.store.get_oauth_client(client_id).await {
        Ok(Some((c, _))) => c,
        Ok(None) => {
            return problem_page("This app isn't registered with mcp-forge (unknown client_id).")
        }
        Err(e) => {
            tracing::error!(error = %e, "authorize");
            return problem_page("Internal error; see the server log.");
        }
    };
    let redirect_uri = match q.redirect_uri.as_deref() {
        Some(r) if client.redirect_uris.iter().any(|u| u == r) => r.to_owned(),
        // One registered URI may be implied (OAuth 2.1 still requires an
        // exact match when given).
        None if client.redirect_uris.len() == 1 => client.redirect_uris[0].clone(),
        _ => return problem_page("The app's redirect URI doesn't match what it registered."),
    };
    let state = q.state.as_deref();
    if q.response_type.as_deref() != Some("code") {
        return redirect_error(&redirect_uri, "unsupported_response_type", state, &iss);
    }
    let challenge = match (
        q.code_challenge.as_deref(),
        q.code_challenge_method.as_deref(),
    ) {
        (Some(c), Some("S256")) if (43..=128).contains(&c.len()) => c.to_owned(),
        _ => return redirect_error(&redirect_uri, "invalid_request", state, &iss),
    };
    let Some(slug) = q.resource.as_deref().and_then(|r| resource_slug(r, &iss)) else {
        return redirect_error(&redirect_uri, "invalid_target", state, &iss);
    };
    let project = match s.store.get_project_by_slug(slug).await {
        Ok(p) => p,
        Err(_) => return redirect_error(&redirect_uri, "invalid_target", state, &iss),
    };
    let id = random_token(24);
    s.authz.add_request(
        id.clone(),
        PendingRequest {
            client_id: client.client_id,
            client_name: client.name,
            redirect_uri,
            state: q.state,
            challenge,
            project_id: project.id,
            started: Instant::now(),
        },
    );
    (
        StatusCode::SEE_OTHER,
        [(LOCATION, format!("/#/oauth/consent/{id}"))],
    )
        .into_response()
}

// ---- consent (web UI) --------------------------------------------------------

/// Approving an app for a project needs write on it.
async fn ensure_write(
    s: &AppState,
    user: Option<&Extension<CurrentUser>>,
    project_id: Uuid,
) -> Result<(), ApiError> {
    let user = user.map(|Extension(u)| u);
    if crate::permissions::access(&s.store, user, project_id).await?
        == Some(forge_store::Access::Write)
    {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

async fn consent_info(
    State(s): State<AppState>,
    user: Option<Extension<CurrentUser>>,
    ApiPath(id): ApiPath<String>,
) -> Result<Json<Value>, ApiError> {
    let r = s
        .authz
        .request(&id)
        .ok_or(ApiError::Store(StoreError::NotFound("sign-in request")))?;
    ensure_write(&s, user.as_ref(), r.project_id).await?;
    let project = s.store.get_project(r.project_id).await?;
    let redirect_host = url::Url::parse(&r.redirect_uri)
        .ok()
        .and_then(|u| {
            u.host_str()
                .map(str::to_owned)
                .or_else(|| Some(u.scheme().to_owned()))
        })
        .unwrap_or_default();
    Ok(Json(json!({
        "client_name": r.client_name,
        "redirect_host": redirect_host,
        "project": { "id": project.id, "name": project.name, "slug": project.slug },
    })))
}

fn decider(user: Option<Extension<CurrentUser>>) -> Option<String> {
    user.map(|Extension(u)| u.user.username)
}

async fn approve(
    State(s): State<AppState>,
    headers: HeaderMap,
    user: Option<Extension<CurrentUser>>,
    ApiPath(id): ApiPath<String>,
) -> Result<Json<Value>, ApiError> {
    let pending = s
        .authz
        .request(&id)
        .ok_or(ApiError::Store(StoreError::NotFound("sign-in request")))?;
    ensure_write(&s, user.as_ref(), pending.project_id).await?;
    let r = s
        .authz
        .take_request(&id)
        .ok_or(ApiError::Store(StoreError::NotFound("sign-in request")))?;
    let code = random_token(32);
    s.authz.add_code(
        code.clone(),
        IssuedCode {
            client_id: r.client_id,
            redirect_uri: r.redirect_uri.clone(),
            challenge: r.challenge,
            project_id: r.project_id,
            approved_by: decider(user),
            issued: Instant::now(),
        },
    );
    let mut u = url::Url::parse(&r.redirect_uri)
        .map_err(|_| ApiError::BadRequest("the app's redirect URI is invalid".into()))?;
    {
        let mut q = u.query_pairs_mut();
        q.append_pair("code", &code);
        if let Some(st) = &r.state {
            q.append_pair("state", st);
        }
        q.append_pair("iss", &origin(&headers));
    }
    Ok(Json(json!({ "redirect": u.as_str() })))
}

async fn deny(
    State(s): State<AppState>,
    headers: HeaderMap,
    user: Option<Extension<CurrentUser>>,
    ApiPath(id): ApiPath<String>,
) -> Result<Json<Value>, ApiError> {
    let pending = s
        .authz
        .request(&id)
        .ok_or(ApiError::Store(StoreError::NotFound("sign-in request")))?;
    ensure_write(&s, user.as_ref(), pending.project_id).await?;
    let r = s
        .authz
        .take_request(&id)
        .ok_or(ApiError::Store(StoreError::NotFound("sign-in request")))?;
    let resp = redirect_error(
        &r.redirect_uri,
        "access_denied",
        r.state.as_deref(),
        &origin(&headers),
    );
    let location = resp
        .headers()
        .get(LOCATION)
        .and_then(|l| l.to_str().ok())
        .unwrap_or("/")
        .to_owned();
    Ok(Json(json!({ "redirect": location })))
}

// ---- token & revoke ----------------------------------------------------------

#[derive(Deserialize)]
struct TokenForm {
    grant_type: Option<String>,
    code: Option<String>,
    redirect_uri: Option<String>,
    code_verifier: Option<String>,
    refresh_token: Option<String>,
    client_id: Option<String>,
    client_secret: Option<String>,
    token: Option<String>,
}

/// `client_id` and secret from HTTP Basic, else the form.
fn client_credentials(
    headers: &HeaderMap,
    form: &TokenForm,
) -> (Option<String>, Option<String>, bool) {
    let basic = headers
        .get(AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Basic "))
        .and_then(|b| decode_basic(b.trim()));
    match basic {
        Some((id, secret)) => (Some(id), Some(secret), true),
        None => (form.client_id.clone(), form.client_secret.clone(), false),
    }
}

fn decode_basic(b64: &str) -> Option<(String, String)> {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut bits = 0u32;
    let mut n = 0;
    let mut out = Vec::new();
    for c in b64.bytes().filter(|&c| c != b'=') {
        let v = T.iter().position(|&t| t == c)? as u32;
        bits = (bits << 6) | v;
        n += 6;
        if n >= 8 {
            n -= 8;
            out.push((bits >> n) as u8);
            bits &= (1 << n) - 1;
        }
    }
    let text = String::from_utf8(out).ok()?;
    let (id, secret) = text.split_once(':')?;
    let dec = |s: &str| {
        url::form_urlencoded::parse(format!("x={s}").as_bytes())
            .next()
            .map(|(_, v)| v.into_owned())
    };
    Some((dec(id)?, dec(secret)?))
}

/// Checks the client's identity. `Err` is the response to send (boxed:
/// a `Response` is large, and the happy path shouldn't carry its size).
async fn authenticate_client(
    s: &AppState,
    headers: &HeaderMap,
    form: &TokenForm,
) -> Result<String, Box<Response>> {
    let (id, secret, via_basic) = client_credentials(headers, form);
    let invalid = || {
        let mut r = oauth_error(
            StatusCode::UNAUTHORIZED,
            "invalid_client",
            "unknown client or wrong secret",
        );
        if via_basic {
            r.headers_mut()
                .insert(WWW_AUTHENTICATE, HeaderValue::from_static("Basic"));
        }
        r
    };
    let Some(id) = id else {
        return Err(Box::new(invalid()));
    };
    let (client, secret_hash) = match s.store.get_oauth_client(&id).await {
        Ok(Some(c)) => c,
        Ok(None) => return Err(Box::new(invalid())),
        Err(e) => {
            tracing::error!(error = %e, "token: client lookup");
            return Err(Box::new(oauth_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                "internal error",
            )));
        }
    };
    if let Some(expected) = secret_hash {
        let ok = secret.is_some_and(|s| {
            constant_time_eq(forge_store::key_hash(&s).as_bytes(), expected.as_bytes())
        });
        if !ok {
            return Err(Box::new(invalid()));
        }
    }
    Ok(client.client_id)
}

fn token_response(t: &IssuedTokens) -> Response {
    let mut resp = Json(json!({
        "access_token": t.access_token,
        "token_type": "Bearer",
        "expires_in": access_ttl().num_seconds(),
        "refresh_token": t.refresh_token,
        "scope": "mcp",
    }))
    .into_response();
    resp.headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp
}

async fn token(
    State(s): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<TokenForm>,
) -> Response {
    let client_id = match authenticate_client(&s, &headers, &form).await {
        Ok(id) => id,
        Err(r) => return *r,
    };
    let invalid_grant = |why: &str| oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", why);
    let issued = match form.grant_type.as_deref() {
        Some("authorization_code") => {
            let (Some(code), Some(verifier)) =
                (form.code.as_deref(), form.code_verifier.as_deref())
            else {
                return oauth_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    "code and code_verifier are required",
                );
            };
            let Some(c) = s.authz.take_code(code) else {
                return invalid_grant("the code is unknown, used or expired");
            };
            if c.client_id != client_id {
                return invalid_grant("the code was issued to another client");
            }
            if form
                .redirect_uri
                .as_deref()
                .is_some_and(|r| r != c.redirect_uri)
            {
                return invalid_grant("redirect_uri doesn't match the authorization request");
            }
            if !constant_time_eq(pkce_challenge(verifier).as_bytes(), c.challenge.as_bytes()) {
                return invalid_grant("code_verifier doesn't match the code challenge");
            }
            s.store
                .create_oauth_grant(
                    &client_id,
                    c.project_id,
                    c.approved_by.as_deref(),
                    access_ttl(),
                    refresh_ttl(),
                )
                .await
                .map(Some)
        }
        Some("refresh_token") => {
            let Some(refresh) = form.refresh_token.as_deref() else {
                return oauth_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    "refresh_token is required",
                );
            };
            s.store
                .refresh_oauth_grant(refresh, &client_id, access_ttl(), refresh_ttl())
                .await
        }
        _ => {
            return oauth_error(
                StatusCode::BAD_REQUEST,
                "unsupported_grant_type",
                "use authorization_code or refresh_token",
            )
        }
    };
    match issued {
        Ok(Some(t)) => token_response(&t),
        Ok(None) => invalid_grant("the refresh token is unknown, expired or revoked"),
        Err(e) => {
            tracing::error!(error = %e, "token");
            oauth_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                "internal error",
            )
        }
    }
}

async fn revoke(
    State(s): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<TokenForm>,
) -> Response {
    if let Err(r) = authenticate_client(&s, &headers, &form).await {
        return *r;
    }
    if let Some(t) = form.token.as_deref() {
        if let Err(e) = s.store.revoke_oauth_token(t).await {
            tracing::error!(error = %e, "revoke");
            return oauth_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                "internal error",
            );
        }
    }
    StatusCode::OK.into_response()
}

// ---- connected apps (web UI) -------------------------------------------------

async fn list_grants(
    State(s): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
) -> Result<Json<Vec<OAuthGrant>>, ApiError> {
    Ok(Json(s.store.list_oauth_grants(project_id).await?))
}

async fn delete_grant(
    State(s): State<AppState>,
    ApiPath((project_id, grant_id)): ApiPath<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    s.store.delete_oauth_grant(project_id, grant_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
#[path = "../tests/unit/authz.rs"]
mod tests;
