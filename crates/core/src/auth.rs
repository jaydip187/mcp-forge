use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::secret::SecretRef;
use crate::validate::{is_valid_name, ValidationErrors, MAX_NAME_LEN};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiKeyLocation {
    Header,
    Query,
}

/// How the client authenticates itself at an OAuth2 token endpoint.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OAuthClientAuth {
    /// `client_id` and `client_secret` in the form body (`client_secret_post`).
    #[default]
    Body,
    /// HTTP Basic with `client_id:client_secret` (`client_secret_basic`).
    Basic,
}

/// How a credential is attached to upstream requests. The secret itself comes
/// from the config's `secret_ref` at call time (for OAuth2: the client secret).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthKind {
    /// Secret sent as header or query parameter `name`.
    ApiKey {
        location: ApiKeyLocation,
        name: String,
    },
    /// `Authorization: Bearer <secret>`.
    Bearer,
    /// `Authorization: Basic base64(username:secret)`.
    Basic { username: String },
    /// OAuth2 client credentials grant: mcp-forge gets a token itself and
    /// sends it as a bearer token.
    #[serde(rename = "oauth2_client_credentials")]
    OAuth2ClientCredentials {
        token_url: String,
        client_id: String,
        #[serde(default)]
        scopes: Vec<String>,
        /// Sent as `audience` (Auth0 and others) when set.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        audience: Option<String>,
        #[serde(default)]
        client_auth: OAuthClientAuth,
    },
    /// OAuth2 authorization code grant with PKCE: a person connects the
    /// config once in the web UI; mcp-forge keeps and refreshes the tokens.
    #[serde(rename = "oauth2_authorization_code")]
    OAuth2AuthorizationCode {
        auth_url: String,
        token_url: String,
        client_id: String,
        #[serde(default)]
        scopes: Vec<String>,
        #[serde(default)]
        client_auth: OAuthClientAuth,
    },
}

impl AuthKind {
    pub fn is_oauth(&self) -> bool {
        matches!(
            self,
            AuthKind::OAuth2ClientCredentials { .. } | AuthKind::OAuth2AuthorizationCode { .. }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthConfig {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub kind: AuthKind,
    pub secret_ref: SecretRef,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Create/replace payload. `secret_ref` is validated during deserialization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthConfigInput {
    pub name: String,
    pub kind: AuthKind,
    pub secret_ref: SecretRef,
}

impl AuthConfigInput {
    pub fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errs = ValidationErrors::default();
        if !is_valid_name(&self.name) {
            errs.push(
                "name",
                format!("must be 1–{MAX_NAME_LEN} characters of a-z, A-Z, 0-9, _ or -"),
            );
        }
        match &self.kind {
            AuthKind::ApiKey { name, .. } => {
                if !is_http_token(name) {
                    errs.push(
                        "kind.name",
                        "must be a non-empty header or query parameter name",
                    );
                }
            }
            AuthKind::Basic { username } => {
                if username.is_empty() || username.contains(':') {
                    errs.push("kind.username", "must be non-empty and contain no `:`");
                }
            }
            AuthKind::Bearer => {}
            AuthKind::OAuth2ClientCredentials {
                token_url,
                client_id,
                scopes,
                audience,
                ..
            } => {
                check_oauth_url(&mut errs, "kind.token_url", token_url);
                check_oauth_common(&mut errs, client_id, scopes);
                if audience.as_ref().is_some_and(|a| a.trim().is_empty()) {
                    errs.push("kind.audience", "must not be blank; omit it instead");
                }
            }
            AuthKind::OAuth2AuthorizationCode {
                auth_url,
                token_url,
                client_id,
                scopes,
                ..
            } => {
                check_oauth_url(&mut errs, "kind.auth_url", auth_url);
                check_oauth_url(&mut errs, "kind.token_url", token_url);
                check_oauth_common(&mut errs, client_id, scopes);
            }
        }
        errs.into_result()
    }
}

/// OAuth endpoints must be HTTPS, except on loopback (local test servers).
fn check_oauth_url(errs: &mut ValidationErrors, field: &str, raw: &str) {
    let ok = match url::Url::parse(raw) {
        Ok(u) => {
            let local = matches!(u.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
            u.has_host()
                && u.username().is_empty()
                && u.password().is_none()
                && u.fragment().is_none()
                && (u.scheme() == "https" || (u.scheme() == "http" && local))
        }
        Err(_) => false,
    };
    if !ok {
        errs.push(
            field,
            "must be an https:// URL without credentials or #fragment (http:// only for localhost)",
        );
    }
}

fn check_oauth_common(errs: &mut ValidationErrors, client_id: &str, scopes: &[String]) {
    if client_id.trim().is_empty() || client_id.len() > 512 {
        errs.push("kind.client_id", "must be 1–512 characters");
    }
    if scopes.len() > 100 {
        errs.push("kind.scopes", "at most 100 scopes");
    }
    for (i, s) in scopes.iter().enumerate() {
        // RFC 6749 scope-token: printable ASCII except space, `"` and `\`.
        if s.is_empty()
            || s.len() > 512
            || !s
                .bytes()
                .all(|b| (0x21..=0x7e).contains(&b) && b != b'"' && b != b'\\')
        {
            errs.push(
                format!("kind.scopes[{i}]"),
                "must be a scope without spaces or quotes",
            );
        }
    }
}

/// RFC 7230 token characters, which also covers sane query parameter names.
fn is_http_token(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 256
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

#[cfg(test)]
#[path = "../tests/unit/auth.rs"]
mod tests;
