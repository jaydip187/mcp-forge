use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::{now, parse_id, Store, StoreError};

/// Tokens of a connected OAuth2 authorization-code config (decrypted).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthToken {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    /// Scopes the provider granted, space-separated, if it said.
    pub scope: Option<String>,
    pub connected_at: DateTime<Utc>,
    pub connected_by: Option<String>,
}

/// What the web UI shows about a connection: never the tokens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OAuthStatus {
    pub auth_config_id: Uuid,
    pub connected_at: DateTime<Utc>,
    pub connected_by: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub scope: Option<String>,
    pub can_refresh: bool,
}

#[derive(sqlx::FromRow)]
struct TokenRow {
    access_token: String,
    refresh_token: Option<String>,
    expires_at: Option<DateTime<Utc>>,
    scope: Option<String>,
    connected_at: DateTime<Utc>,
    connected_by: Option<String>,
}

#[derive(sqlx::FromRow)]
struct StatusRow {
    auth_config_id: String,
    connected_at: DateTime<Utc>,
    connected_by: Option<String>,
    expires_at: Option<DateTime<Utc>>,
    scope: Option<String>,
    has_refresh: bool,
}

impl Store {
    pub async fn get_oauth_token(&self, auth_id: Uuid) -> Result<Option<OAuthToken>, StoreError> {
        let row = sqlx::query_as::<_, TokenRow>(
            "SELECT access_token, refresh_token, expires_at, scope, connected_at, connected_by \
             FROM oauth_tokens WHERE auth_config_id = ?",
        )
        .bind(auth_id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        row.map(|r| {
            Ok(OAuthToken {
                access_token: self.cipher.open(&r.access_token)?,
                refresh_token: r
                    .refresh_token
                    .as_deref()
                    .map(|t| self.cipher.open(t))
                    .transpose()?,
                expires_at: r.expires_at,
                scope: r.scope,
                connected_at: r.connected_at,
                connected_by: r.connected_by,
            })
        })
        .transpose()
    }

    /// Stores (or replaces) the tokens of an auth config.
    pub async fn put_oauth_token(&self, auth_id: Uuid, t: &OAuthToken) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO oauth_tokens (auth_config_id, access_token, refresh_token, expires_at, scope, \
             connected_at, connected_by, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT (auth_config_id) DO UPDATE SET access_token = excluded.access_token, \
             refresh_token = excluded.refresh_token, expires_at = excluded.expires_at, \
             scope = excluded.scope, connected_at = excluded.connected_at, \
             connected_by = excluded.connected_by, updated_at = excluded.updated_at",
        )
        .bind(auth_id.to_string())
        .bind(self.cipher.seal(&t.access_token))
        .bind(t.refresh_token.as_deref().map(|r| self.cipher.seal(r)))
        .bind(t.expires_at)
        .bind(&t.scope)
        .bind(t.connected_at)
        .bind(&t.connected_by)
        .bind(now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_oauth_token(&self, auth_id: Uuid) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM oauth_tokens WHERE auth_config_id = ?")
            .bind(auth_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Connection status of the project's connected OAuth configs.
    pub async fn list_oauth_status(
        &self,
        project_id: Uuid,
    ) -> Result<Vec<OAuthStatus>, StoreError> {
        self.ensure_project(project_id).await?;
        sqlx::query_as::<_, StatusRow>(
            "SELECT t.auth_config_id, t.connected_at, t.connected_by, t.expires_at, t.scope, \
             t.refresh_token IS NOT NULL AS has_refresh \
             FROM oauth_tokens t JOIN auth_configs a ON a.id = t.auth_config_id \
             WHERE a.project_id = ? ORDER BY a.name",
        )
        .bind(project_id.to_string())
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|r| {
            Ok(OAuthStatus {
                auth_config_id: parse_id(&r.auth_config_id)?,
                connected_at: r.connected_at,
                connected_by: r.connected_by,
                expires_at: r.expires_at,
                scope: r.scope,
                can_refresh: r.has_refresh,
            })
        })
        .collect()
    }
}
