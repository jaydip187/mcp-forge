//! Storage for mcp-forge's own OAuth server (sign-in for `/mcp`): registered
//! clients and approved grants. Tokens are kept as SHA-256 only.

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::api_keys::{key_hash, random_secret};
use crate::{from_json, now, parse_id, to_json, Store, StoreError};

/// Access tokens start with this, refresh tokens with [`REFRESH_PREFIX`].
pub const ACCESS_PREFIX: &str = "mcpo_";
pub const REFRESH_PREFIX: &str = "mcpr_";
const CLIENT_PREFIX: &str = "mcpc_";
const SECRET_PREFIX: &str = "mcps_";
/// Registration is open, so the number of clients is capped; old clients
/// that never got a grant make room first.
const MAX_CLIENTS: i64 = 500;
const TOUCH_SECS: i64 = 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OAuthClient {
    pub client_id: String,
    pub name: String,
    pub redirect_uris: Vec<String>,
    /// `none`, `client_secret_post` or `client_secret_basic`.
    pub auth_method: String,
    pub created_at: DateTime<Utc>,
}

/// A connection someone approved, as the UI lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OAuthGrant {
    pub id: Uuid,
    pub client_id: String,
    pub client_name: String,
    pub project_id: Uuid,
    pub approved_by: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub access_expires_at: DateTime<Utc>,
    pub refresh_expires_at: DateTime<Utc>,
}

/// Fresh tokens for a grant (the only time they exist in clear).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedTokens {
    pub grant: OAuthGrant,
    pub access_token: String,
    pub refresh_token: String,
}

#[derive(sqlx::FromRow)]
struct ClientRow {
    client_id: String,
    secret_hash: Option<String>,
    name: String,
    redirect_uris: String,
    auth_method: String,
    created_at: DateTime<Utc>,
}

impl ClientRow {
    fn into_client(self) -> Result<(OAuthClient, Option<String>), StoreError> {
        Ok((
            OAuthClient {
                client_id: self.client_id,
                name: self.name,
                redirect_uris: from_json(&self.redirect_uris, "client redirect URIs")?,
                auth_method: self.auth_method,
                created_at: self.created_at,
            },
            self.secret_hash,
        ))
    }
}

#[derive(sqlx::FromRow)]
struct GrantRow {
    id: String,
    client_id: String,
    client_name: String,
    project_id: String,
    approved_by: Option<String>,
    created_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
    access_expires_at: DateTime<Utc>,
    refresh_expires_at: DateTime<Utc>,
}

impl TryFrom<GrantRow> for OAuthGrant {
    type Error = StoreError;
    fn try_from(r: GrantRow) -> Result<Self, StoreError> {
        Ok(OAuthGrant {
            id: parse_id(&r.id)?,
            client_id: r.client_id,
            client_name: r.client_name,
            project_id: parse_id(&r.project_id)?,
            approved_by: r.approved_by,
            created_at: r.created_at,
            last_used_at: r.last_used_at,
            access_expires_at: r.access_expires_at,
            refresh_expires_at: r.refresh_expires_at,
        })
    }
}

const GRANT_COLUMNS: &str =
    "g.id, g.client_id, c.name AS client_name, g.project_id, g.approved_by, \
     g.created_at, g.last_used_at, g.access_expires_at, g.refresh_expires_at";

impl Store {
    /// Registers a client. Confidential clients get a secret (returned once).
    pub async fn register_oauth_client(
        &self,
        name: &str,
        redirect_uris: &[String],
        auth_method: &str,
    ) -> Result<(OAuthClient, Option<String>), StoreError> {
        let ts = now();
        let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM oauth_clients")
            .fetch_one(&self.pool)
            .await?;
        if count >= MAX_CLIENTS {
            sqlx::query(
                "DELETE FROM oauth_clients WHERE created_at < ? AND client_id NOT IN \
                 (SELECT client_id FROM oauth_grants)",
            )
            .bind(ts - Duration::days(1))
            .execute(&self.pool)
            .await?;
            let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM oauth_clients")
                .fetch_one(&self.pool)
                .await?;
            if count >= MAX_CLIENTS {
                return Err(StoreError::Conflict(
                    "too many registered clients; try again later".into(),
                ));
            }
        }
        let client = OAuthClient {
            client_id: random_secret(CLIENT_PREFIX),
            name: name.to_owned(),
            redirect_uris: redirect_uris.to_vec(),
            auth_method: auth_method.to_owned(),
            created_at: ts,
        };
        let secret = (auth_method != "none").then(|| random_secret(SECRET_PREFIX));
        sqlx::query(
            "INSERT INTO oauth_clients (client_id, secret_hash, name, redirect_uris, auth_method, created_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&client.client_id)
        .bind(secret.as_deref().map(key_hash))
        .bind(&client.name)
        .bind(to_json(&client.redirect_uris))
        .bind(&client.auth_method)
        .bind(client.created_at)
        .execute(&self.pool)
        .await?;
        Ok((client, secret))
    }

    /// The client and its secret's hash (`None` for public clients).
    pub async fn get_oauth_client(
        &self,
        client_id: &str,
    ) -> Result<Option<(OAuthClient, Option<String>)>, StoreError> {
        sqlx::query_as::<_, ClientRow>(
            "SELECT client_id, secret_hash, name, redirect_uris, auth_method, created_at \
             FROM oauth_clients WHERE client_id = ?",
        )
        .bind(client_id)
        .fetch_optional(&self.pool)
        .await?
        .map(ClientRow::into_client)
        .transpose()
    }

    /// Creates a grant with fresh tokens.
    pub async fn create_oauth_grant(
        &self,
        client_id: &str,
        project_id: Uuid,
        approved_by: Option<&str>,
        access_ttl: Duration,
        refresh_ttl: Duration,
    ) -> Result<IssuedTokens, StoreError> {
        let ts = now();
        let id = Uuid::new_v4();
        let access = random_secret(ACCESS_PREFIX);
        let refresh = random_secret(REFRESH_PREFIX);
        sqlx::query(
            "INSERT INTO oauth_grants (id, client_id, project_id, approved_by, access_hash, \
             access_expires_at, refresh_hash, refresh_expires_at, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(client_id)
        .bind(project_id.to_string())
        .bind(approved_by)
        .bind(key_hash(&access))
        .bind(ts + access_ttl)
        .bind(key_hash(&refresh))
        .bind(ts + refresh_ttl)
        .bind(ts)
        .execute(&self.pool)
        .await?;
        Ok(IssuedTokens {
            grant: self
                .grant_where("g.id = ?", &id.to_string())
                .await?
                .ok_or(StoreError::NotFound("grant"))?,
            access_token: access,
            refresh_token: refresh,
        })
    }

    async fn grant_where(&self, cond: &str, value: &str) -> Result<Option<OAuthGrant>, StoreError> {
        sqlx::query_as::<_, GrantRow>(&format!(
            "SELECT {GRANT_COLUMNS} FROM oauth_grants g JOIN oauth_clients c ON c.client_id = g.client_id \
             WHERE {cond}"
        ))
        .bind(value)
        .fetch_optional(&self.pool)
        .await?
        .map(OAuthGrant::try_from)
        .transpose()
    }

    /// The grant an access token belongs to, if it hasn't expired. Records the use.
    pub async fn find_oauth_access(&self, token: &str) -> Result<Option<OAuthGrant>, StoreError> {
        if !token.starts_with(ACCESS_PREFIX) {
            return Ok(None);
        }
        let Some(g) = self
            .grant_where("g.access_hash = ?", &key_hash(token))
            .await?
        else {
            return Ok(None);
        };
        let ts = now();
        if g.access_expires_at <= ts {
            return Ok(None);
        }
        if g.last_used_at
            .is_none_or(|t| ts - t > Duration::seconds(TOUCH_SECS))
        {
            sqlx::query("UPDATE oauth_grants SET last_used_at = ? WHERE id = ?")
                .bind(ts)
                .bind(g.id.to_string())
                .execute(&self.pool)
                .await?;
        }
        Ok(Some(g))
    }

    /// Refresh grant: swaps both tokens (rotation). `None` if the refresh
    /// token is unknown, expired, or belongs to another client.
    pub async fn refresh_oauth_grant(
        &self,
        refresh_token: &str,
        client_id: &str,
        access_ttl: Duration,
        refresh_ttl: Duration,
    ) -> Result<Option<IssuedTokens>, StoreError> {
        let Some(g) = self
            .grant_where("g.refresh_hash = ?", &key_hash(refresh_token))
            .await?
        else {
            return Ok(None);
        };
        let ts = now();
        if g.client_id != client_id || g.refresh_expires_at <= ts {
            return Ok(None);
        }
        let access = random_secret(ACCESS_PREFIX);
        let refresh = random_secret(REFRESH_PREFIX);
        let done = sqlx::query(
            "UPDATE oauth_grants SET access_hash = ?, access_expires_at = ?, refresh_hash = ?, \
             refresh_expires_at = ?, last_used_at = ? WHERE id = ? AND refresh_hash = ?",
        )
        .bind(key_hash(&access))
        .bind(ts + access_ttl)
        .bind(key_hash(&refresh))
        .bind(ts + refresh_ttl)
        .bind(ts)
        .bind(g.id.to_string())
        .bind(key_hash(refresh_token))
        .execute(&self.pool)
        .await?;
        if done.rows_affected() == 0 {
            // Someone else used this refresh token at the same moment.
            return Ok(None);
        }
        Ok(Some(IssuedTokens {
            grant: self
                .grant_where("g.id = ?", &g.id.to_string())
                .await?
                .ok_or(StoreError::NotFound("grant"))?,
            access_token: access,
            refresh_token: refresh,
        }))
    }

    /// Revokes the grant an access or refresh token belongs to (RFC 7009:
    /// unknown tokens are not an error).
    pub async fn revoke_oauth_token(&self, token: &str) -> Result<bool, StoreError> {
        let h = key_hash(token);
        let done =
            sqlx::query("DELETE FROM oauth_grants WHERE access_hash = ? OR refresh_hash = ?")
                .bind(&h)
                .bind(&h)
                .execute(&self.pool)
                .await?;
        Ok(done.rows_affected() > 0)
    }

    pub async fn list_oauth_grants(&self, project_id: Uuid) -> Result<Vec<OAuthGrant>, StoreError> {
        self.ensure_project(project_id).await?;
        sqlx::query_as::<_, GrantRow>(&format!(
            "SELECT {GRANT_COLUMNS} FROM oauth_grants g JOIN oauth_clients c ON c.client_id = g.client_id \
             WHERE g.project_id = ? ORDER BY g.created_at DESC"
        ))
        .bind(project_id.to_string())
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(OAuthGrant::try_from)
        .collect()
    }

    pub async fn delete_oauth_grant(&self, project_id: Uuid, id: Uuid) -> Result<(), StoreError> {
        let done = sqlx::query("DELETE FROM oauth_grants WHERE project_id = ? AND id = ?")
            .bind(project_id.to_string())
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        if done.rows_affected() == 0 {
            return Err(StoreError::NotFound("connected app"));
        }
        Ok(())
    }
}
