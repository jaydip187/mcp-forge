use chrono::{DateTime, Utc};
use forge_core::{AuthConfig, AuthConfigInput, SecretRef};
use uuid::Uuid;

use crate::{from_json, now, on_unique, parse_id, to_json, Store, StoreError};

#[derive(sqlx::FromRow)]
struct AuthRow {
    id: String,
    project_id: String,
    name: String,
    kind: String,
    secret_ref: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl TryFrom<AuthRow> for AuthConfig {
    type Error = StoreError;
    fn try_from(r: AuthRow) -> Result<Self, StoreError> {
        Ok(AuthConfig {
            id: parse_id(&r.id)?,
            project_id: parse_id(&r.project_id)?,
            name: r.name,
            kind: from_json(&r.kind, "auth kind")?,
            secret_ref: SecretRef::parse(&r.secret_ref)
                .map_err(|e| StoreError::Corrupt(format!("bad secret_ref: {e}")))?,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })
    }
}

const COLUMNS: &str = "id, project_id, name, kind, secret_ref, created_at, updated_at";

fn name_taken(name: &str) -> impl FnOnce() -> String + '_ {
    move || format!("an auth config named `{name}` already exists in this project")
}

impl Store {
    pub async fn create_auth_config(
        &self,
        project_id: Uuid,
        input: &AuthConfigInput,
    ) -> Result<AuthConfig, StoreError> {
        input.validate()?;
        self.ensure_project(project_id).await?;
        let ts = now();
        let cfg = AuthConfig {
            id: Uuid::new_v4(),
            project_id,
            name: input.name.clone(),
            kind: input.kind.clone(),
            secret_ref: input.secret_ref.clone(),
            created_at: ts,
            updated_at: ts,
        };
        sqlx::query(
            "INSERT INTO auth_configs (id, project_id, name, kind, secret_ref, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(cfg.id.to_string())
        .bind(project_id.to_string())
        .bind(&cfg.name)
        .bind(to_json(&cfg.kind))
        .bind(cfg.secret_ref.as_str())
        .bind(cfg.created_at)
        .bind(cfg.updated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| on_unique(e, name_taken(&cfg.name)))?;
        Ok(cfg)
    }

    pub async fn list_auth_configs(&self, project_id: Uuid) -> Result<Vec<AuthConfig>, StoreError> {
        self.ensure_project(project_id).await?;
        sqlx::query_as::<_, AuthRow>(&format!(
            "SELECT {COLUMNS} FROM auth_configs WHERE project_id = ? ORDER BY name"
        ))
        .bind(project_id.to_string())
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(AuthConfig::try_from)
        .collect()
    }

    pub async fn get_auth_config(
        &self,
        project_id: Uuid,
        id: Uuid,
    ) -> Result<AuthConfig, StoreError> {
        sqlx::query_as::<_, AuthRow>(&format!(
            "SELECT {COLUMNS} FROM auth_configs WHERE project_id = ? AND id = ?"
        ))
        .bind(project_id.to_string())
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .ok_or(StoreError::NotFound("auth config"))?
        .try_into()
    }

    pub async fn update_auth_config(
        &self,
        project_id: Uuid,
        id: Uuid,
        input: &AuthConfigInput,
    ) -> Result<AuthConfig, StoreError> {
        input.validate()?;
        let mut cfg = self.get_auth_config(project_id, id).await?;
        cfg.name = input.name.clone();
        cfg.kind = input.kind.clone();
        cfg.secret_ref = input.secret_ref.clone();
        cfg.updated_at = now();
        sqlx::query(
            "UPDATE auth_configs SET name = ?, kind = ?, secret_ref = ?, updated_at = ? \
             WHERE project_id = ? AND id = ?",
        )
        .bind(&cfg.name)
        .bind(to_json(&cfg.kind))
        .bind(cfg.secret_ref.as_str())
        .bind(cfg.updated_at)
        .bind(project_id.to_string())
        .bind(id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| on_unique(e, name_taken(&cfg.name)))?;
        Ok(cfg)
    }

    /// Fails with `Conflict` while any tool still references the config.
    pub async fn delete_auth_config(&self, project_id: Uuid, id: Uuid) -> Result<(), StoreError> {
        let users: Vec<(String,)> =
            sqlx::query_as("SELECT name FROM tools WHERE auth_ref = ? ORDER BY name")
                .bind(id.to_string())
                .fetch_all(&self.pool)
                .await?;
        if !users.is_empty() {
            let names: Vec<String> = users.into_iter().map(|(n,)| n).collect();
            return Err(StoreError::Conflict(format!(
                "auth config is used by tools: {}",
                names.join(", ")
            )));
        }
        let done = sqlx::query("DELETE FROM auth_configs WHERE project_id = ? AND id = ?")
            .bind(project_id.to_string())
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(|e| match &e {
                // A tool grabbed the config between the check and the delete.
                sqlx::Error::Database(db) if db.is_foreign_key_violation() => {
                    StoreError::Conflict("auth config is used by tools".into())
                }
                _ => StoreError::Db(e),
            })?;
        if done.rows_affected() == 0 {
            return Err(StoreError::NotFound("auth config"));
        }
        Ok(())
    }
}
