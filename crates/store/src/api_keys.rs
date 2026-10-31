use chrono::{DateTime, Duration, Utc};
use forge_core::{is_valid_name, ValidationErrors, MAX_NAME_LEN};
use ring::digest::{digest, SHA256};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{from_json, now, on_unique, parse_id, to_json, Store, StoreError};

/// Every key starts with this, so it's recognisable (and scannable) as an
/// mcp-forge key.
pub const KEY_PREFIX: &str = "mcpf_";
/// Characters of the key kept in clear to recognise it in lists.
const SHOWN_CHARS: usize = 12;
/// `last_used_at` is only rewritten when older than this, to spare writes.
const TOUCH_SECS: i64 = 60;

/// An access key for one project's `/mcp` endpoint (never the key itself).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApiKey {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    /// The key's first characters (`mcpf_AbCdEfG`), to tell keys apart.
    pub prefix: String,
    /// Tools the key may list and call; `None` = every tool agents can see.
    pub tools: Option<Vec<String>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub created_by: Option<String>,
    pub last_used_at: Option<DateTime<Utc>>,
}

impl ApiKey {
    pub fn expired(&self, now: DateTime<Utc>) -> bool {
        self.expires_at.is_some_and(|e| e <= now)
    }

    pub fn allows(&self, tool: &str) -> bool {
        self.tools
            .as_ref()
            .is_none_or(|t| t.iter().any(|n| n == tool))
    }
}

/// Create payload.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ApiKeyInput {
    pub name: String,
    #[serde(default)]
    pub tools: Option<Vec<String>>,
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
}

impl ApiKeyInput {
    pub fn validate(&self, now: DateTime<Utc>) -> Result<(), ValidationErrors> {
        let mut errs = ValidationErrors::default();
        if !is_valid_name(&self.name) {
            errs.push(
                "name",
                format!("must be 1–{MAX_NAME_LEN} characters of a-z, A-Z, 0-9, _ or -"),
            );
        }
        if let Some(tools) = &self.tools {
            if tools.is_empty() {
                errs.push("tools", "pick at least one tool, or allow all tools");
            }
            for (i, t) in tools.iter().enumerate() {
                if !is_valid_name(t) {
                    errs.push(format!("tools[{i}]"), "is not a valid tool name");
                }
            }
        }
        if self.expires_at.is_some_and(|e| e <= now) {
            errs.push("expires_at", "must be in the future");
        }
        errs.into_result()
    }
}

/// SHA-256 of a key, hex. Keys carry 256 random bits, so a fast hash is
/// enough (no password hashing needed).
pub fn key_hash(key: &str) -> String {
    digest(&SHA256, key.as_bytes())
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn new_key() -> String {
    let mut bytes = [0u8; 32];
    SystemRandom::new()
        .fill(&mut bytes)
        .expect("system randomness");
    // URL-safe base64 without padding.
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::from(KEY_PREFIX);
    let mut acc: u32 = 0;
    let mut bits = 0;
    for b in bytes {
        acc = (acc << 8) | u32::from(b);
        bits += 8;
        while bits >= 6 {
            bits -= 6;
            out.push(T[((acc >> bits) & 63) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(T[((acc << (6 - bits)) & 63) as usize] as char);
    }
    out
}

#[derive(sqlx::FromRow)]
struct KeyRow {
    id: String,
    project_id: String,
    name: String,
    prefix: String,
    tools: Option<String>,
    expires_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    created_by: Option<String>,
    last_used_at: Option<DateTime<Utc>>,
}

impl TryFrom<KeyRow> for ApiKey {
    type Error = StoreError;
    fn try_from(r: KeyRow) -> Result<Self, StoreError> {
        Ok(ApiKey {
            id: parse_id(&r.id)?,
            project_id: parse_id(&r.project_id)?,
            name: r.name,
            prefix: r.prefix,
            tools: r
                .tools
                .as_deref()
                .map(|t| from_json(t, "api key tools"))
                .transpose()?,
            expires_at: r.expires_at,
            created_at: r.created_at,
            created_by: r.created_by,
            last_used_at: r.last_used_at,
        })
    }
}

const COLUMNS: &str =
    "id, project_id, name, prefix, tools, expires_at, created_at, created_by, last_used_at";

impl Store {
    /// Creates a key; the returned string is the key itself, shown once.
    pub async fn create_api_key(
        &self,
        project_id: Uuid,
        input: &ApiKeyInput,
        created_by: Option<&str>,
    ) -> Result<(ApiKey, String), StoreError> {
        let ts = now();
        input.validate(ts)?;
        self.ensure_project(project_id).await?;
        let secret = new_key();
        let key = ApiKey {
            id: Uuid::new_v4(),
            project_id,
            name: input.name.clone(),
            prefix: secret.chars().take(SHOWN_CHARS).collect(),
            tools: input.tools.clone(),
            expires_at: input.expires_at,
            created_at: ts,
            created_by: created_by.map(str::to_owned),
            last_used_at: None,
        };
        sqlx::query(&format!(
            "INSERT INTO api_keys ({COLUMNS}, key_hash) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        ))
        .bind(key.id.to_string())
        .bind(project_id.to_string())
        .bind(&key.name)
        .bind(&key.prefix)
        .bind(key.tools.as_ref().map(to_json))
        .bind(key.expires_at)
        .bind(key.created_at)
        .bind(&key.created_by)
        .bind(key.last_used_at)
        .bind(key_hash(&secret))
        .execute(&self.pool)
        .await
        .map_err(|e| {
            on_unique(e, || {
                format!(
                    "a key named `{}` already exists in this project",
                    input.name
                )
            })
        })?;
        Ok((key, secret))
    }

    pub async fn list_api_keys(&self, project_id: Uuid) -> Result<Vec<ApiKey>, StoreError> {
        self.ensure_project(project_id).await?;
        sqlx::query_as::<_, KeyRow>(&format!(
            "SELECT {COLUMNS} FROM api_keys WHERE project_id = ? ORDER BY name COLLATE NOCASE"
        ))
        .bind(project_id.to_string())
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(ApiKey::try_from)
        .collect()
    }

    pub async fn delete_api_key(&self, project_id: Uuid, id: Uuid) -> Result<(), StoreError> {
        let done = sqlx::query("DELETE FROM api_keys WHERE project_id = ? AND id = ?")
            .bind(project_id.to_string())
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        if done.rows_affected() == 0 {
            return Err(StoreError::NotFound("access key"));
        }
        Ok(())
    }

    /// The key `secret` belongs to, if any (expired keys included: the
    /// caller decides). Records the use.
    pub async fn find_api_key(&self, secret: &str) -> Result<Option<ApiKey>, StoreError> {
        if !secret.starts_with(KEY_PREFIX) {
            return Ok(None);
        }
        let row = sqlx::query_as::<_, KeyRow>(&format!(
            "SELECT {COLUMNS} FROM api_keys WHERE key_hash = ?"
        ))
        .bind(key_hash(secret))
        .fetch_optional(&self.pool)
        .await?;
        let Some(key) = row.map(ApiKey::try_from).transpose()? else {
            return Ok(None);
        };
        let ts = now();
        if key
            .last_used_at
            .is_none_or(|t| ts - t > Duration::seconds(TOUCH_SECS))
        {
            sqlx::query("UPDATE api_keys SET last_used_at = ? WHERE id = ?")
                .bind(ts)
                .bind(key.id.to_string())
                .execute(&self.pool)
                .await?;
        }
        Ok(Some(key))
    }
}
