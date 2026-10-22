use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::{now, on_unique, parse_id, Store, StoreError};

/// An account that can sign in to the web UI and REST API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct User {
    pub id: Uuid,
    pub username: String,
    pub created_at: DateTime<Utc>,
}

/// A stored session, looked up by the SHA-256 of its cookie token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub user: User,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct UserRow {
    id: String,
    username: String,
    created_at: DateTime<Utc>,
}

impl TryFrom<UserRow> for User {
    type Error = StoreError;
    fn try_from(r: UserRow) -> Result<Self, StoreError> {
        Ok(User {
            id: parse_id(&r.id)?,
            username: r.username,
            created_at: r.created_at,
        })
    }
}

#[derive(sqlx::FromRow)]
struct SessionRow {
    id: String,
    username: String,
    user_created_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
    last_seen_at: DateTime<Utc>,
}

impl Store {
    pub async fn count_users(&self) -> Result<i64, StoreError> {
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(&self.pool)
            .await?;
        Ok(n)
    }

    pub async fn list_users(&self) -> Result<Vec<User>, StoreError> {
        sqlx::query_as::<_, UserRow>(
            "SELECT id, username, created_at FROM users ORDER BY username COLLATE NOCASE",
        )
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(User::try_from)
        .collect()
    }

    /// `password_hash` is an already-computed PHC string; the store never sees passwords.
    pub async fn create_user(
        &self,
        username: &str,
        password_hash: &str,
    ) -> Result<User, StoreError> {
        let user = User {
            id: Uuid::new_v4(),
            username: username.to_owned(),
            created_at: now(),
        };
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, created_at, password_changed_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(user.id.to_string())
        .bind(&user.username)
        .bind(password_hash)
        .bind(user.created_at)
        .bind(user.created_at)
        .execute(&self.pool)
        .await
        .map_err(|e| on_unique(e, || format!("user `{username}` already exists")))?;
        Ok(user)
    }

    /// The user and their password hash, matching the name case-insensitively.
    pub async fn find_user_credentials(
        &self,
        username: &str,
    ) -> Result<Option<(User, String)>, StoreError> {
        let row: Option<(String, String, DateTime<Utc>, String)> = sqlx::query_as(
            "SELECT id, username, created_at, password_hash FROM users WHERE username = ?",
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|(id, username, created_at, hash)| {
            Ok((
                User {
                    id: parse_id(&id)?,
                    username,
                    created_at,
                },
                hash,
            ))
        })
        .transpose()
    }

    pub async fn password_hash(&self, user_id: Uuid) -> Result<String, StoreError> {
        sqlx::query_scalar("SELECT password_hash FROM users WHERE id = ?")
            .bind(user_id.to_string())
            .fetch_optional(&self.pool)
            .await?
            .ok_or(StoreError::NotFound("user"))
    }

    /// Replaces the password and ends every session of the user except `keep`
    /// (the token hash of the session that made the change, if any).
    pub async fn set_password(
        &self,
        user_id: Uuid,
        password_hash: &str,
        keep: Option<&str>,
    ) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await?;
        let done =
            sqlx::query("UPDATE users SET password_hash = ?, password_changed_at = ? WHERE id = ?")
                .bind(password_hash)
                .bind(now())
                .bind(user_id.to_string())
                .execute(&mut *tx)
                .await?;
        if done.rows_affected() == 0 {
            return Err(StoreError::NotFound("user"));
        }
        sqlx::query("DELETE FROM sessions WHERE user_id = ? AND token_hash IS NOT ?")
            .bind(user_id.to_string())
            .bind(keep)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Deletes the user; their sessions go with them.
    pub async fn delete_user(&self, username: &str) -> Result<(), StoreError> {
        let done = sqlx::query("DELETE FROM users WHERE username = ?")
            .bind(username)
            .execute(&self.pool)
            .await?;
        if done.rows_affected() == 0 {
            return Err(StoreError::NotFound("user"));
        }
        Ok(())
    }

    pub async fn create_session(&self, token_hash: &str, user_id: Uuid) -> Result<(), StoreError> {
        let ts = now();
        sqlx::query(
            "INSERT INTO sessions (token_hash, user_id, created_at, last_seen_at) VALUES (?, ?, ?, ?)",
        )
        .bind(token_hash)
        .bind(user_id.to_string())
        .bind(ts)
        .bind(ts)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn find_session(&self, token_hash: &str) -> Result<Option<Session>, StoreError> {
        let row = sqlx::query_as::<_, SessionRow>(
            "SELECT u.id, u.username, u.created_at AS user_created_at, s.created_at, s.last_seen_at \
             FROM sessions s JOIN users u ON u.id = s.user_id WHERE s.token_hash = ?",
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|r| {
            Ok(Session {
                user: User {
                    id: parse_id(&r.id)?,
                    username: r.username,
                    created_at: r.user_created_at,
                },
                created_at: r.created_at,
                last_seen_at: r.last_seen_at,
            })
        })
        .transpose()
    }

    pub async fn touch_session(
        &self,
        token_hash: &str,
        at: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        sqlx::query("UPDATE sessions SET last_seen_at = ? WHERE token_hash = ?")
            .bind(at)
            .bind(token_hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_session(&self, token_hash: &str) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
            .bind(token_hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Removes sessions created before `created_before` or idle since `seen_before`.
    pub async fn purge_sessions(
        &self,
        created_before: DateTime<Utc>,
        seen_before: DateTime<Utc>,
    ) -> Result<u64, StoreError> {
        let done = sqlx::query("DELETE FROM sessions WHERE created_at < ? OR last_seen_at < ?")
            .bind(created_before)
            .bind(seen_before)
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected())
    }
}
