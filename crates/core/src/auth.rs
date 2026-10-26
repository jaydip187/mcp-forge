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

/// How a credential is attached to upstream requests. The secret itself comes
/// from the config's `secret_ref` at call time.
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
        }
        errs.into_result()
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
