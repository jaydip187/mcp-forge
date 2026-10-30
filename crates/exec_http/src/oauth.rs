//! OAuth2 for upstream APIs: token requests (client credentials, code
//! exchange, refresh), PKCE, the authorize URL, and an in-memory token cache.
//! Stored tokens of the authorization code flow live in the store; the
//! caller (forge-mcp) connects the two.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, Utc};
use forge_core::{AuthConfig, AuthKind, OAuthClientAuth};
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use reqwest::Url;
use ring::digest::{digest, SHA256};
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::Value;
use uuid::Uuid;

use crate::{base64_encode, redact_all, ExecError, HttpExecutor};

/// Tokens are refreshed this long before they expire.
const EXPIRY_MARGIN_SECS: i64 = 60;
/// Assumed lifetime when the provider doesn't say (`expires_in` missing).
const DEFAULT_LIFETIME_SECS: i64 = 3600;
/// Token endpoint answers bigger than this are refused.
const MAX_TOKEN_RESPONSE_BYTES: usize = 64 * 1024;

/// A token endpoint's successful answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub scope: Option<String>,
}

/// Whether a token expiring at `expires_at` is still good to use at `now`.
pub fn usable(expires_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    expires_at.is_none_or(|e| e - Duration::seconds(EXPIRY_MARGIN_SECS) > now)
}

/// URL-safe base64 without padding (PKCE, state).
fn base64url(bytes: &[u8]) -> String {
    base64_encode(bytes)
        .trim_end_matches('=')
        .replace('+', "-")
        .replace('/', "_")
}

/// `n` random bytes as URL-safe base64.
pub fn random_token(n: usize) -> String {
    let mut bytes = vec![0u8; n];
    SystemRandom::new()
        .fill(&mut bytes)
        .expect("system randomness");
    base64url(&bytes)
}

/// A PKCE verifier and its S256 challenge.
pub fn pkce_pair() -> (String, String) {
    let verifier = random_token(32);
    let challenge = pkce_challenge(&verifier);
    (verifier, challenge)
}

pub fn pkce_challenge(verifier: &str) -> String {
    base64url(digest(&SHA256, verifier.as_bytes()).as_ref())
}

/// Where to send the person to connect an authorization code config.
pub fn authorize_url(
    cfg: &AuthConfig,
    redirect_uri: &str,
    state: &str,
    challenge: &str,
) -> Result<Url, ExecError> {
    let AuthKind::OAuth2AuthorizationCode {
        auth_url,
        client_id,
        scopes,
        ..
    } = &cfg.kind
    else {
        return Err(ExecError::Config(format!(
            "auth config `{}` doesn't use the authorization code flow",
            cfg.name
        )));
    };
    let mut url = Url::parse(auth_url)
        .map_err(|e| ExecError::Config(format!("auth config `{}`: auth URL: {e}", cfg.name)))?;
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("response_type", "code")
            .append_pair("client_id", client_id)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("state", state)
            .append_pair("code_challenge", challenge)
            .append_pair("code_challenge_method", "S256");
        if !scopes.is_empty() {
            q.append_pair("scope", &scopes.join(" "));
        }
    }
    Ok(url)
}

fn form(pairs: &[(&str, &str)]) -> String {
    let mut s = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in pairs {
        s.append_pair(k, v);
    }
    s.finish()
}

impl HttpExecutor {
    /// Client credentials grant: a fresh token for `cfg`.
    pub async fn client_credentials_token(
        &self,
        cfg: &AuthConfig,
    ) -> Result<TokenResponse, ExecError> {
        let AuthKind::OAuth2ClientCredentials {
            scopes, audience, ..
        } = &cfg.kind
        else {
            return Err(ExecError::Config(format!(
                "auth config `{}` doesn't use client credentials",
                cfg.name
            )));
        };
        let scope = scopes.join(" ");
        let mut pairs = vec![("grant_type", "client_credentials")];
        if !scope.is_empty() {
            pairs.push(("scope", &scope));
        }
        if let Some(a) = audience {
            pairs.push(("audience", a));
        }
        self.token_request(cfg, &pairs, &[]).await
    }

    /// Authorization code grant: trades the code from the callback for tokens.
    pub async fn exchange_code(
        &self,
        cfg: &AuthConfig,
        code: &str,
        verifier: &str,
        redirect_uri: &str,
    ) -> Result<TokenResponse, ExecError> {
        let pairs = [
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("code_verifier", verifier),
        ];
        self.token_request(cfg, &pairs, &[code, verifier]).await
    }

    /// Refresh grant.
    pub async fn refresh_oauth(
        &self,
        cfg: &AuthConfig,
        refresh_token: &str,
    ) -> Result<TokenResponse, ExecError> {
        let pairs = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
        ];
        self.token_request(cfg, &pairs, &[refresh_token]).await
    }

    /// POSTs a form to the config's token endpoint, authenticating the client
    /// with its secret. `extra_secrets` are redacted from errors too.
    async fn token_request(
        &self,
        cfg: &AuthConfig,
        pairs: &[(&str, &str)],
        extra_secrets: &[&str],
    ) -> Result<TokenResponse, ExecError> {
        let (token_url, client_id, client_auth) = match &cfg.kind {
            AuthKind::OAuth2ClientCredentials {
                token_url,
                client_id,
                client_auth,
                ..
            }
            | AuthKind::OAuth2AuthorizationCode {
                token_url,
                client_id,
                client_auth,
                ..
            } => (token_url, client_id, *client_auth),
            _ => {
                return Err(ExecError::Config(format!(
                    "auth config `{}` isn't an OAuth2 config",
                    cfg.name
                )))
            }
        };
        let secret = self.resolve_secret(cfg).await?;
        let mut secrets: Vec<String> = vec![secret.clone()];
        secrets.extend(extra_secrets.iter().map(|s| (*s).to_owned()));
        let mut all: Vec<(&str, &str)> = pairs.to_vec();
        let mut req = self
            .client
            .post(token_url)
            .header(ACCEPT, "application/json");
        match client_auth {
            OAuthClientAuth::Body => {
                all.push(("client_id", client_id));
                all.push(("client_secret", &secret));
            }
            OAuthClientAuth::Basic => {
                // RFC 6749 §2.3.1: form-encode both parts before joining.
                let enc = |s: &str| form(&[("x", s)])[2..].to_owned();
                let token =
                    base64_encode(format!("{}:{}", enc(client_id), enc(&secret)).as_bytes());
                secrets.push(token.clone());
                req = req.header(AUTHORIZATION, format!("Basic {token}"));
            }
        }
        let redact = |s: &str| redact_all(s, &secrets);
        let host = Url::parse(token_url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
            .unwrap_or_default();
        let resp = req
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(form(&all))
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| {
                ExecError::Request(redact(&format!(
                    "OAuth token request to {host} failed: {}",
                    e.without_url()
                )))
            })?;
        let status = resp.status();
        let body = resp.bytes().await.map_err(|e| {
            ExecError::Request(format!(
                "reading the OAuth token response from {host}: {}",
                e.without_url()
            ))
        })?;
        if body.len() > MAX_TOKEN_RESPONSE_BYTES {
            return Err(ExecError::Request(format!(
                "the OAuth token response from {host} is too large"
            )));
        }
        let json: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        let field = |k: &str| json.get(k).and_then(Value::as_str).map(str::to_owned);
        if !status.is_success() {
            let detail = match (field("error"), field("error_description")) {
                (Some(e), Some(d)) => format!("{e} ({d})"),
                (Some(e), None) => e,
                _ => String::from_utf8_lossy(&body).chars().take(300).collect(),
            };
            return Err(ExecError::Config(redact(&format!(
                "auth config `{}`: the token endpoint at {host} answered {}: {detail}",
                cfg.name,
                status.as_u16()
            ))));
        }
        let Some(access_token) = field("access_token").filter(|t| !t.is_empty()) else {
            return Err(ExecError::Config(format!(
                "auth config `{}`: the token endpoint at {host} returned no access_token",
                cfg.name
            )));
        };
        if let Some(t) = field("token_type").filter(|t| !t.eq_ignore_ascii_case("bearer")) {
            return Err(ExecError::Config(format!(
                "auth config `{}`: token type `{t}` isn't supported (only bearer tokens)",
                cfg.name
            )));
        }
        let lifetime = json
            .get("expires_in")
            .and_then(|v| v.as_i64().or_else(|| v.as_str()?.parse().ok()));
        Ok(TokenResponse {
            access_token,
            refresh_token: field("refresh_token").filter(|t| !t.is_empty()),
            expires_at: lifetime.map(|s| Utc::now() + Duration::seconds(s.max(0))),
            scope: field("scope"),
        })
    }
}

/// A cache entry is keyed by the config and its last edit, so editing a
/// config never reuses a token from its old settings.
type CacheKey = (Uuid, DateTime<Utc>);

#[derive(Clone)]
struct Cached {
    token: String,
    expires_at: DateTime<Utc>,
}

/// Access tokens in memory, fetched at most once at a time per config.
#[derive(Clone, Default)]
pub struct TokenCache {
    tokens: Arc<Mutex<HashMap<CacheKey, Cached>>>,
    locks: Arc<Mutex<HashMap<CacheKey, Arc<tokio::sync::Mutex<()>>>>>,
}

impl TokenCache {
    /// The cached token for `cfg`, or a new one from `fetch`. With `stale`
    /// (a token the upstream just rejected) a cached copy of that same token
    /// is fetched again; a different one (someone else already refreshed) is
    /// returned as is.
    pub async fn get_or_fetch<F, Fut>(
        &self,
        cfg: &AuthConfig,
        stale: Option<&str>,
        fetch: F,
    ) -> Result<String, ExecError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<(String, Option<DateTime<Utc>>), ExecError>>,
    {
        let key = (cfg.id, cfg.updated_at);
        let lock = self
            .locks
            .lock()
            .expect("token locks poisoned")
            .entry(key)
            .or_default()
            .clone();
        let _one_at_a_time = lock.lock().await;
        let now = Utc::now();
        if let Some(c) = self.tokens.lock().expect("token cache poisoned").get(&key) {
            if usable(Some(c.expires_at), now) && Some(c.token.as_str()) != stale {
                return Ok(c.token.clone());
            }
        }
        let (token, expires_at) = fetch().await?;
        let expires_at =
            expires_at.unwrap_or_else(|| Utc::now() + Duration::seconds(DEFAULT_LIFETIME_SECS));
        self.tokens.lock().expect("token cache poisoned").insert(
            key,
            Cached {
                token: token.clone(),
                expires_at,
            },
        );
        Ok(token)
    }

    /// Forgets every token of an auth config (disconnected or reconnected).
    pub fn forget(&self, auth_id: Uuid) {
        self.tokens
            .lock()
            .expect("token cache poisoned")
            .retain(|(id, _), _| *id != auth_id);
        self.locks
            .lock()
            .expect("token locks poisoned")
            .retain(|(id, _), _| *id != auth_id);
    }
}

#[cfg(test)]
#[path = "../tests/unit/oauth.rs"]
mod tests;
