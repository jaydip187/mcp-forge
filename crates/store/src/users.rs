use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{now, on_unique, parse_id, Store, StoreError};

/// What an account may do across mcp-forge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Everything, including users and creating/deleting projects.
    Admin,
    /// Only the projects granted to them, with read or write each.
    Member,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Member => "member",
        }
    }

    fn parse(s: &str) -> Result<Self, StoreError> {
        match s {
            "admin" => Ok(Role::Admin),
            "member" => Ok(Role::Member),
            other => Err(StoreError::Corrupt(format!("bad role {other:?}"))),
        }
    }
}

/// A member's permission on one project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    Read,
    Write,
}

impl Access {
    pub fn as_str(self) -> &'static str {
        match self {
            Access::Read => "read",
            Access::Write => "write",
        }
    }

    fn parse(s: &str) -> Result<Self, StoreError> {
        match s {
            "read" => Ok(Access::Read),
            "write" => Ok(Access::Write),
            other => Err(StoreError::Corrupt(format!("bad access level {other:?}"))),
        }
    }
}

/// An account that can sign in to the web UI and REST API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct User {
    pub id: Uuid,
    pub username: String,
    pub role: Role,
    pub created_at: DateTime<Utc>,
}

/// Who can see a project, for its People card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectPerson {
    pub user: User,
    /// `None` for admins (they can do everything).
    pub access: Option<Access>,
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
    role: String,
    created_at: DateTime<Utc>,
}

impl TryFrom<UserRow> for User {
    type Error = StoreError;
    fn try_from(r: UserRow) -> Result<Self, StoreError> {
        Ok(User {
            id: parse_id(&r.id)?,
            username: r.username,
            role: Role::parse(&r.role)?,
            created_at: r.created_at,
        })
    }
}

#[derive(sqlx::FromRow)]
struct SessionRow {
    id: String,
    username: String,
    role: String,
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
            "SELECT id, username, role, created_at FROM users ORDER BY username COLLATE NOCASE",
        )
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(User::try_from)
        .collect()
    }

    /// Creates an admin. `password_hash` is an already-computed PHC string;
    /// the store never sees passwords.
    pub async fn create_user(
        &self,
        username: &str,
        password_hash: &str,
    ) -> Result<User, StoreError> {
        self.create_user_as(username, password_hash, Role::Admin)
            .await
    }

    pub async fn create_user_as(
        &self,
        username: &str,
        password_hash: &str,
        role: Role,
    ) -> Result<User, StoreError> {
        let user = User {
            id: Uuid::new_v4(),
            username: username.to_owned(),
            role,
            created_at: now(),
        };
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, created_at, password_changed_at, role) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(user.id.to_string())
        .bind(&user.username)
        .bind(password_hash)
        .bind(user.created_at)
        .bind(user.created_at)
        .bind(role.as_str())
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
        let row: Option<(String, String, String, DateTime<Utc>, String)> = sqlx::query_as(
            "SELECT id, username, role, created_at, password_hash FROM users WHERE username = ?",
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|(id, username, role, created_at, hash)| {
            Ok((
                User {
                    id: parse_id(&id)?,
                    username,
                    role: Role::parse(&role)?,
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

    /// Deletes the user; their sessions go with them. The last admin can't go.
    pub async fn delete_user(&self, username: &str) -> Result<(), StoreError> {
        let target = sqlx::query_as::<_, UserRow>(
            "SELECT id, username, role, created_at FROM users WHERE username = ?",
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await?
        .map(User::try_from)
        .transpose()?
        .ok_or(StoreError::NotFound("user"))?;
        if target.role == Role::Admin && self.count_admins().await? <= 1 {
            return Err(StoreError::Conflict(
                "can't remove the last admin; make someone else admin first".into(),
            ));
        }
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
            "SELECT u.id, u.username, u.role, u.created_at AS user_created_at, s.created_at, s.last_seen_at \
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
                    role: Role::parse(&r.role)?,
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

/// Sign-in backoff for one key (`user:<name>` or `addr:<ip>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backoff {
    pub key: String,
    pub failures: u32,
    pub last_failure_at: DateTime<Utc>,
    pub locked_until: Option<DateTime<Utc>>,
}

#[derive(sqlx::FromRow)]
struct BackoffRow {
    key: String,
    failures: i64,
    last_failure_at: DateTime<Utc>,
    locked_until: Option<DateTime<Utc>>,
}

impl From<BackoffRow> for Backoff {
    fn from(r: BackoffRow) -> Self {
        Backoff {
            key: r.key,
            failures: u32::try_from(r.failures).unwrap_or(u32::MAX),
            last_failure_at: r.last_failure_at,
            locked_until: r.locked_until,
        }
    }
}

impl Store {
    pub async fn get_backoff(&self, key: &str) -> Result<Option<Backoff>, StoreError> {
        let row = sqlx::query_as::<_, BackoffRow>(
            "SELECT key, failures, last_failure_at, locked_until FROM login_backoff WHERE key = ?",
        )
        .bind(key)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(Backoff::from))
    }

    pub async fn put_backoff(&self, b: &Backoff) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO login_backoff (key, failures, last_failure_at, locked_until) \
             VALUES (?, ?, ?, ?) \
             ON CONFLICT (key) DO UPDATE SET failures = excluded.failures, \
             last_failure_at = excluded.last_failure_at, locked_until = excluded.locked_until",
        )
        .bind(&b.key)
        .bind(i64::from(b.failures))
        .bind(b.last_failure_at)
        .bind(b.locked_until)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_backoff(&self, key: &str) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM login_backoff WHERE key = ?")
            .bind(key)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Removes entries whose last failure is before `before` and that are no
    /// longer locked at `now`.
    pub async fn prune_backoff(
        &self,
        before: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<u64, StoreError> {
        let done = sqlx::query(
            "DELETE FROM login_backoff WHERE last_failure_at < ? \
             AND (locked_until IS NULL OR locked_until < ?)",
        )
        .bind(before)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected())
    }
}

impl Store {
    pub async fn count_admins(&self) -> Result<i64, StoreError> {
        Ok(
            sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE role = 'admin'")
                .fetch_one(&self.pool)
                .await?,
        )
    }

    pub async fn get_user(&self, id: Uuid) -> Result<User, StoreError> {
        sqlx::query_as::<_, UserRow>(
            "SELECT id, username, role, created_at FROM users WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .ok_or(StoreError::NotFound("user"))?
        .try_into()
    }

    /// Changes a role; the last admin can't be demoted.
    pub async fn set_user_role(&self, id: Uuid, role: Role) -> Result<User, StoreError> {
        let user = self.get_user(id).await?;
        if user.role == Role::Admin && role != Role::Admin && self.count_admins().await? <= 1 {
            return Err(StoreError::Conflict(
                "can't demote the last admin; make someone else admin first".into(),
            ));
        }
        sqlx::query("UPDATE users SET role = ? WHERE id = ?")
            .bind(role.as_str())
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(User { role, ..user })
    }

    /// Grants `access` on a project, or removes it with `None`.
    pub async fn set_project_access(
        &self,
        user_id: Uuid,
        project_id: Uuid,
        access: Option<Access>,
    ) -> Result<(), StoreError> {
        self.get_user(user_id).await?;
        self.ensure_project(project_id).await?;
        match access {
            Some(a) => {
                sqlx::query(
                    "INSERT INTO project_access (user_id, project_id, level) VALUES (?, ?, ?) \
                     ON CONFLICT (user_id, project_id) DO UPDATE SET level = excluded.level",
                )
                .bind(user_id.to_string())
                .bind(project_id.to_string())
                .bind(a.as_str())
                .execute(&self.pool)
                .await?;
            }
            None => {
                sqlx::query("DELETE FROM project_access WHERE user_id = ? AND project_id = ?")
                    .bind(user_id.to_string())
                    .bind(project_id.to_string())
                    .execute(&self.pool)
                    .await?;
            }
        }
        Ok(())
    }

    /// A member's permission on a project (admins aren't listed here).
    pub async fn project_access(
        &self,
        user_id: Uuid,
        project_id: Uuid,
    ) -> Result<Option<Access>, StoreError> {
        let level: Option<String> = sqlx::query_scalar(
            "SELECT level FROM project_access WHERE user_id = ? AND project_id = ?",
        )
        .bind(user_id.to_string())
        .bind(project_id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        level.as_deref().map(Access::parse).transpose()
    }

    /// Every project a member has access to.
    pub async fn user_access(&self, user_id: Uuid) -> Result<Vec<(Uuid, Access)>, StoreError> {
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT project_id, level FROM project_access WHERE user_id = ?")
                .bind(user_id.to_string())
                .fetch_all(&self.pool)
                .await?;
        rows.into_iter()
            .map(|(p, l)| Ok((parse_id(&p)?, Access::parse(&l)?)))
            .collect()
    }

    /// Admins plus members with access to the project.
    pub async fn project_people(&self, project_id: Uuid) -> Result<Vec<ProjectPerson>, StoreError> {
        self.ensure_project(project_id).await?;
        #[derive(sqlx::FromRow)]
        struct PersonRow {
            #[sqlx(flatten)]
            user: UserRow,
            level: Option<String>,
        }
        sqlx::query_as::<_, PersonRow>(
            "SELECT u.id, u.username, u.role, u.created_at, a.level FROM users u \
             LEFT JOIN project_access a ON a.user_id = u.id AND a.project_id = ? \
             WHERE u.role = 'admin' OR a.level IS NOT NULL ORDER BY u.username COLLATE NOCASE",
        )
        .bind(project_id.to_string())
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|r| {
            let user = User::try_from(r.user)?;
            Ok(ProjectPerson {
                access: if user.role == Role::Admin {
                    None
                } else {
                    r.level.as_deref().map(Access::parse).transpose()?
                },
                user,
            })
        })
        .collect()
    }
}
