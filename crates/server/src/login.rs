//! Accounts for the web UI and REST API: first-run setup, sign-in and sessions.
//!
//! - With no account yet, the server logs a one-time setup code; whoever
//!   holds it creates the first account (`POST /api/auth/setup`).
//! - Sessions live in an `HttpOnly`, `SameSite=Strict` cookie holding a random
//!   token; the store keeps only its SHA-256. They end after 7 idle days or
//!   30 days in all, on sign-out, or when the password changes.
//! - Failed attempts back off per username and per client address.
//! - Accounts are also managed offline with `mcp-forge user …`.

mod limiter;
mod password;

use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::Instant;

use argon2::password_hash::rand_core::{OsRng, RngCore};
use axum::extract::{ConnectInfo, FromRequestParts, State};
use axum::http::header::{COOKIE, RETRY_AFTER, SET_COOKIE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use chrono::{DateTime, Duration, Utc};
use forge_core::ValidationErrors;
use forge_store::{Store, StoreError, User};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::api::{ApiError, ApiJson};
use crate::security::error;
use crate::AppState;

use limiter::Limiter;
pub use password::{check_password, check_username, hash_password};
use password::{verify_dummy, verify_password};

pub const SESSION_COOKIE: &str = "mcp_forge_session";
/// Required on state-changing `/api` calls made with the session cookie.
/// Browsers only send custom headers cross-origin after a CORS preflight,
/// which this server never grants, so other sites can't forge these calls.
pub const CSRF_HEADER: &str = "x-mcp-forge";

/// Paths under `/api` reachable without a session.
const PUBLIC_PATHS: &[&str] = &[
    "/api/auth/status",
    "/api/auth/setup",
    "/api/auth/login",
    "/api/auth/logout",
];

/// Failed attempts allowed before backoff starts.
const USER_FREE_ATTEMPTS: u32 = 5;
/// Higher, so one person can't lock out everyone behind the same address.
const ADDR_FREE_ATTEMPTS: u32 = 20;

fn idle_timeout() -> Duration {
    Duration::days(7)
}

fn max_lifetime() -> Duration {
    Duration::days(30)
}

/// `last_seen_at` is only rewritten when older than this, to spare writes.
fn touch_interval() -> Duration {
    Duration::minutes(1)
}

/// Whether a session created at `created` and last used at `last_seen` is still valid.
pub fn session_live(created: DateTime<Utc>, last_seen: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    now - created < max_lifetime() && now - last_seen < idle_timeout()
}

/// The signed-in user of a request, set by the security guard.
#[derive(Clone, Debug)]
pub struct CurrentUser {
    pub user: User,
    /// SHA-256 of the session token, to keep this session on password change.
    pub token_hash: String,
}

pub struct Login {
    setup_code: Mutex<Option<String>>,
    users: Limiter,
    addrs: Limiter,
}

impl Default for Login {
    fn default() -> Self {
        Self {
            setup_code: Mutex::default(),
            users: Limiter::new(USER_FREE_ATTEMPTS),
            addrs: Limiter::new(ADDR_FREE_ATTEMPTS),
        }
    }
}

impl Login {
    /// The one-time code for creating the first account, while there is no
    /// account. Generated (and logged) on first call.
    pub async fn setup_code(&self, store: &Store) -> Result<Option<String>, StoreError> {
        if store.count_users().await? > 0 {
            return Ok(None);
        }
        let mut slot = self.setup_code.lock().expect("setup code lock");
        let code = slot.get_or_insert_with(|| {
            let code = new_setup_code();
            tracing::warn!(
                "no accounts yet: open the web UI and create the first one with setup code {code}"
            );
            code
        });
        Ok(Some(code.clone()))
    }

    /// Takes the setup code if `given` matches it, so only one caller wins.
    fn take_setup_code(&self, given: &str) -> bool {
        let mut slot = self.setup_code.lock().expect("setup code lock");
        let given = normalize_code(given);
        if slot
            .as_deref()
            .is_some_and(|c| crate::security::constant_time_eq(c.as_bytes(), given.as_bytes()))
        {
            *slot = None;
            true
        } else {
            false
        }
    }

    fn restore_setup_code(&self, code: &str) {
        *self.setup_code.lock().expect("setup code lock") = Some(normalize_code(code));
    }

    /// `Err(seconds)` to wait while any of `keys` is locked out.
    fn check_limits(&self, keys: &[String]) -> Result<(), u64> {
        let now = Instant::now();
        for (limiter, key) in self.limiters(keys) {
            limiter
                .check(key, now)
                .map_err(|wait| wait.as_secs().max(1))?;
        }
        Ok(())
    }

    fn record_failure(&self, keys: &[String]) {
        let now = Instant::now();
        for (limiter, key) in self.limiters(keys) {
            limiter.fail(key, now);
        }
    }

    fn record_success(&self, keys: &[String]) {
        for (limiter, key) in self.limiters(keys) {
            limiter.succeed(key);
        }
    }

    /// Keys are `user:<name>` or `addr:<ip>`.
    fn limiters<'a>(&'a self, keys: &'a [String]) -> impl Iterator<Item = (&'a Limiter, &'a str)> {
        keys.iter().map(move |k| {
            let limiter = if k.starts_with("user:") {
                &self.users
            } else {
                &self.addrs
            };
            (limiter, k.as_str())
        })
    }
}

/// Twelve characters from an unambiguous alphabet, as `XXXX-XXXX-XXXX` (60 bits).
fn new_setup_code() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut bytes = [0u8; 12];
    OsRng.fill_bytes(&mut bytes);
    let chars: Vec<char> = bytes
        .iter()
        .map(|b| ALPHABET[usize::from(*b) % ALPHABET.len()] as char)
        .collect();
    chars
        .chunks(4)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("-")
}

/// Upper-cases and re-groups a code typed with or without dashes or spaces.
fn normalize_code(code: &str) -> String {
    let chars: Vec<char> = code
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .collect();
    chars
        .chunks(4)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("-")
}

/// A random 256-bit session token, hex-encoded.
fn new_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

pub fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub fn is_public(path: &str) -> bool {
    PUBLIC_PATHS.contains(&path)
}

/// Value of the cookie `name` in the request's `Cookie` headers.
pub fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v)
}

/// The session's user, if the request carries a live session cookie.
pub async fn authenticate(
    store: &Store,
    headers: &HeaderMap,
) -> Result<Option<CurrentUser>, StoreError> {
    let Some(token) = cookie(headers, SESSION_COOKIE) else {
        return Ok(None);
    };
    if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Ok(None);
    }
    let hash = token_hash(token);
    let Some(session) = store.find_session(&hash).await? else {
        return Ok(None);
    };
    let now = Utc::now();
    if !session_live(session.created_at, session.last_seen_at, now) {
        store.delete_session(&hash).await?;
        return Ok(None);
    }
    if now - session.last_seen_at > touch_interval() {
        store.touch_session(&hash, now).await?;
    }
    Ok(Some(CurrentUser {
        user: session.user,
        token_hash: hash,
    }))
}

fn session_cookie(headers: &HeaderMap, token: &str, max_age: i64) -> HeaderValue {
    // Behind a TLS-terminating proxy the cookie should only travel over HTTPS.
    let secure = headers
        .get("x-forwarded-proto")
        .is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"https"));
    let value = format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age}{}",
        if secure { "; Secure" } else { "" }
    );
    HeaderValue::from_str(&value).expect("cookie is ASCII")
}

/// Client address, when the server was started with connection info.
pub struct ClientAddr(Option<IpAddr>);

impl<S: Send + Sync> FromRequestParts<S> for ClientAddr {
    type Rejection = std::convert::Infallible;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(Self(
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|c| c.0.ip()),
        ))
    }
}

impl ClientAddr {
    fn key(&self) -> Option<String> {
        self.0.map(|ip| format!("addr:{ip}"))
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/auth/status", get(status))
        .route("/auth/setup", post(setup))
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/auth/password", post(change_password))
}

fn too_many(retry_after: u64) -> Response {
    let mut resp = error(
        StatusCode::TOO_MANY_REQUESTS,
        "too_many_attempts",
        &format!("Too many failed attempts. Try again in {retry_after} seconds."),
    );
    resp.headers_mut()
        .insert(RETRY_AFTER, HeaderValue::from(retry_after));
    resp
}

fn invalid(field: &str, msg: String) -> ApiError {
    ApiError::Store(StoreError::Validation(ValidationErrors::single(field, msg)))
}

async fn hash_blocking(password: String) -> String {
    tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .expect("hashing task")
}

async fn verify_blocking(password: String, phc: Option<String>) -> bool {
    tokio::task::spawn_blocking(move || match phc {
        Some(phc) => verify_password(&password, &phc),
        None => {
            verify_dummy(&password);
            false
        }
    })
    .await
    .expect("verify task")
}

/// Starts a session: stores its hash, answers with the cookie and the user.
async fn start_session(
    s: &AppState,
    headers: &HeaderMap,
    user: User,
    status: StatusCode,
) -> Result<Response, ApiError> {
    let token = new_token();
    s.store.create_session(&token_hash(&token), user.id).await?;
    let now = Utc::now();
    s.store
        .purge_sessions(now - max_lifetime(), now - idle_timeout())
        .await?;
    let mut resp = (status, Json(json!({ "user": user }))).into_response();
    resp.headers_mut().insert(
        SET_COOKIE,
        session_cookie(headers, &token, max_lifetime().num_seconds()),
    );
    Ok(resp)
}

async fn status(State(s): State<AppState>, headers: HeaderMap) -> Result<Response, ApiError> {
    let setup_required = s.login.setup_code(&s.store).await?.is_some();
    let user = authenticate(&s.store, &headers).await?.map(|c| c.user);
    Ok(Json(json!({
        "login_required": s.security.require_login,
        "setup_required": setup_required,
        "user": user,
    }))
    .into_response())
}

#[derive(Deserialize)]
struct SetupReq {
    code: String,
    username: String,
    password: String,
}

async fn setup(
    State(s): State<AppState>,
    addr: ClientAddr,
    headers: HeaderMap,
    ApiJson(req): ApiJson<SetupReq>,
) -> Result<Response, ApiError> {
    let keys: Vec<String> = addr.key().into_iter().collect();
    if let Err(wait) = s.login.check_limits(&keys) {
        return Ok(too_many(wait));
    }
    if s.login.setup_code(&s.store).await?.is_none() {
        return Err(ApiError::Store(StoreError::Conflict(
            "an account already exists; sign in instead".into(),
        )));
    }
    let username = req.username.trim().to_owned();
    check_username(&username).map_err(|m| invalid("username", m))?;
    check_password(&req.password).map_err(|m| invalid("password", m))?;
    if !s.login.take_setup_code(&req.code) {
        s.login.record_failure(&keys);
        return Ok(error(
            StatusCode::FORBIDDEN,
            "invalid_setup_code",
            "That setup code is wrong. Copy it from the server's log output.",
        ));
    }
    let phc = hash_blocking(req.password).await;
    let user = match s.store.create_user(&username, &phc).await {
        Ok(u) => u,
        Err(e) => {
            s.login.restore_setup_code(&req.code);
            return Err(e.into());
        }
    };
    s.login.record_success(&keys);
    tracing::info!(username = %user.username, "first account created");
    start_session(&s, &headers, user, StatusCode::CREATED).await
}

#[derive(Deserialize)]
struct LoginReq {
    username: String,
    password: String,
}

async fn login(
    State(s): State<AppState>,
    addr: ClientAddr,
    headers: HeaderMap,
    ApiJson(req): ApiJson<LoginReq>,
) -> Result<Response, ApiError> {
    let username = req.username.trim();
    let mut keys = vec![format!("user:{}", username.to_lowercase())];
    keys.extend(addr.key());
    if let Err(wait) = s.login.check_limits(&keys) {
        return Ok(too_many(wait));
    }
    let found = s.store.find_user_credentials(username).await?;
    let (user, phc) = match found {
        Some((u, h)) => (Some(u), Some(h)),
        None => (None, None),
    };
    let ok = verify_blocking(req.password, phc).await;
    let Some(user) = user.filter(|_| ok) else {
        s.login.record_failure(&keys);
        return Ok(error(
            StatusCode::UNAUTHORIZED,
            "invalid_credentials",
            "Wrong username or password.",
        ));
    };
    s.login.record_success(&keys);
    start_session(&s, &headers, user, StatusCode::OK).await
}

async fn logout(State(s): State<AppState>, headers: HeaderMap) -> Result<Response, ApiError> {
    if let Some(token) = cookie(&headers, SESSION_COOKIE) {
        s.store.delete_session(&token_hash(token)).await?;
    }
    let mut resp = StatusCode::NO_CONTENT.into_response();
    resp.headers_mut()
        .insert(SET_COOKIE, session_cookie(&headers, "", 0));
    Ok(resp)
}

#[derive(Deserialize)]
struct PasswordReq {
    current_password: String,
    new_password: String,
}

async fn change_password(
    State(s): State<AppState>,
    current: Option<Extension<CurrentUser>>,
    ApiJson(req): ApiJson<PasswordReq>,
) -> Result<Response, ApiError> {
    let Some(Extension(current)) = current else {
        return Ok(error(
            StatusCode::UNAUTHORIZED,
            "sign_in_required",
            "Sign in to change your password.",
        ));
    };
    let keys = vec![format!("user:{}", current.user.username.to_lowercase())];
    if let Err(wait) = s.login.check_limits(&keys) {
        return Ok(too_many(wait));
    }
    let phc = s.store.password_hash(current.user.id).await?;
    if !verify_blocking(req.current_password, Some(phc)).await {
        s.login.record_failure(&keys);
        return Err(invalid(
            "current_password",
            "doesn't match your current password".into(),
        ));
    }
    check_password(&req.new_password).map_err(|m| invalid("new_password", m))?;
    let phc = hash_blocking(req.new_password).await;
    s.store
        .set_password(current.user.id, &phc, Some(&current.token_hash))
        .await?;
    s.login.record_success(&keys);
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[cfg(test)]
#[path = "../tests/unit/login.rs"]
mod tests;
