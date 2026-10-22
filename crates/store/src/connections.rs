use chrono::{DateTime, Utc};
use forge_core::{AccessMode, ColumnMask, DbConnection, DbConnectionInput, Driver, SecretRef};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{from_json, now, on_unique, parse_id, to_json, Store, StoreError};

/// Everything but identity, driver and DSN ref, kept as one JSON column.
#[derive(Serialize, Deserialize)]
struct ConnSettings {
    access_mode: AccessMode,
    row_limit: u32,
    timeout_ms: u32,
    max_affected_rows: u32,
    allow_ddl: bool,
    table_allowlist: Vec<String>,
    column_masks: Vec<ColumnMask>,
}

impl From<&DbConnection> for ConnSettings {
    fn from(c: &DbConnection) -> Self {
        Self {
            access_mode: c.access_mode,
            row_limit: c.row_limit,
            timeout_ms: c.timeout_ms,
            max_affected_rows: c.max_affected_rows,
            allow_ddl: c.allow_ddl,
            table_allowlist: c.table_allowlist.clone(),
            column_masks: c.column_masks.clone(),
        }
    }
}

#[derive(sqlx::FromRow)]
struct ConnRow {
    id: String,
    project_id: String,
    name: String,
    driver: String,
    dsn_ref: String,
    settings: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl TryFrom<ConnRow> for DbConnection {
    type Error = StoreError;
    fn try_from(r: ConnRow) -> Result<Self, StoreError> {
        let s: ConnSettings = from_json(&r.settings, "connection settings")?;
        let driver: Driver = from_json(&format!("\"{}\"", r.driver), "driver")?;
        Ok(DbConnection {
            id: parse_id(&r.id)?,
            project_id: parse_id(&r.project_id)?,
            name: r.name,
            driver,
            dsn_ref: SecretRef::parse(&r.dsn_ref)
                .map_err(|e| StoreError::Corrupt(format!("bad dsn_ref: {e}")))?,
            access_mode: s.access_mode,
            row_limit: s.row_limit,
            timeout_ms: s.timeout_ms,
            max_affected_rows: s.max_affected_rows,
            allow_ddl: s.allow_ddl,
            table_allowlist: s.table_allowlist,
            column_masks: s.column_masks,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })
    }
}

const COLUMNS: &str = "id, project_id, name, driver, dsn_ref, settings, created_at, updated_at";

fn name_taken(name: &str) -> impl FnOnce() -> String + '_ {
    move || format!("a connection named `{name}` already exists in this project")
}

pub(crate) fn new_connection(project_id: Uuid, input: &DbConnectionInput) -> DbConnection {
    let ts = now();
    DbConnection {
        id: Uuid::new_v4(),
        project_id,
        name: input.name.clone(),
        driver: input.driver,
        dsn_ref: input.dsn_ref.clone(),
        access_mode: input.access_mode,
        row_limit: input.row_limit,
        timeout_ms: input.timeout_ms,
        max_affected_rows: input.max_affected_rows,
        allow_ddl: input.allow_ddl,
        table_allowlist: input.table_allowlist.clone(),
        column_masks: input.column_masks.clone(),
        created_at: ts,
        updated_at: ts,
    }
}

pub(crate) async fn insert_connection<'e, E>(exec: E, c: &DbConnection) -> Result<(), StoreError>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    sqlx::query(
        "INSERT INTO db_connections (id, project_id, name, driver, dsn_ref, settings, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(c.id.to_string())
    .bind(c.project_id.to_string())
    .bind(&c.name)
    .bind(c.driver.as_str())
    .bind(c.dsn_ref.as_str())
    .bind(to_json(&ConnSettings::from(c)))
    .bind(c.created_at)
    .bind(c.updated_at)
    .execute(exec)
    .await
    .map_err(|e| on_unique(e, name_taken(&c.name)))?;
    Ok(())
}

impl Store {
    pub async fn create_connection(
        &self,
        project_id: Uuid,
        input: &DbConnectionInput,
    ) -> Result<DbConnection, StoreError> {
        input.validate()?;
        self.ensure_project(project_id).await?;
        let c = new_connection(project_id, input);
        insert_connection(&self.pool, &c).await?;
        Ok(c)
    }

    pub async fn list_connections(
        &self,
        project_id: Uuid,
    ) -> Result<Vec<DbConnection>, StoreError> {
        self.ensure_project(project_id).await?;
        sqlx::query_as::<_, ConnRow>(&format!(
            "SELECT {COLUMNS} FROM db_connections WHERE project_id = ? ORDER BY name"
        ))
        .bind(project_id.to_string())
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(DbConnection::try_from)
        .collect()
    }

    pub async fn get_connection(
        &self,
        project_id: Uuid,
        id: Uuid,
    ) -> Result<DbConnection, StoreError> {
        sqlx::query_as::<_, ConnRow>(&format!(
            "SELECT {COLUMNS} FROM db_connections WHERE project_id = ? AND id = ?"
        ))
        .bind(project_id.to_string())
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .ok_or(StoreError::NotFound("connection"))?
        .try_into()
    }

    /// Replaces a connection. Changing the driver is refused while SQL tools
    /// use it, since their SQL was checked against the old dialect.
    pub async fn update_connection(
        &self,
        project_id: Uuid,
        id: Uuid,
        input: &DbConnectionInput,
    ) -> Result<DbConnection, StoreError> {
        input.validate()?;
        let mut c = self.get_connection(project_id, id).await?;
        if c.driver != input.driver {
            let users = self.connection_users(id).await?;
            if !users.is_empty() {
                return Err(StoreError::Conflict(format!(
                    "can't change the driver while tools use this connection: {}",
                    users.join(", ")
                )));
            }
        }
        c.name = input.name.clone();
        c.driver = input.driver;
        c.dsn_ref = input.dsn_ref.clone();
        c.access_mode = input.access_mode;
        c.row_limit = input.row_limit;
        c.timeout_ms = input.timeout_ms;
        c.max_affected_rows = input.max_affected_rows;
        c.allow_ddl = input.allow_ddl;
        c.table_allowlist = input.table_allowlist.clone();
        c.column_masks = input.column_masks.clone();
        c.updated_at = now();
        sqlx::query(
            "UPDATE db_connections SET name = ?, driver = ?, dsn_ref = ?, settings = ?, updated_at = ? \
             WHERE project_id = ? AND id = ?",
        )
        .bind(&c.name)
        .bind(c.driver.as_str())
        .bind(c.dsn_ref.as_str())
        .bind(to_json(&ConnSettings::from(&c)))
        .bind(c.updated_at)
        .bind(project_id.to_string())
        .bind(id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| on_unique(e, name_taken(&c.name)))?;
        Ok(c)
    }

    async fn connection_users(&self, id: Uuid) -> Result<Vec<String>, StoreError> {
        let users: Vec<(String,)> =
            sqlx::query_as("SELECT name FROM tools WHERE connection_ref = ? ORDER BY name")
                .bind(id.to_string())
                .fetch_all(&self.pool)
                .await?;
        Ok(users.into_iter().map(|(n,)| n).collect())
    }

    /// Fails with `Conflict` while any tool still uses the connection.
    pub async fn delete_connection(&self, project_id: Uuid, id: Uuid) -> Result<(), StoreError> {
        let users = self.connection_users(id).await?;
        if !users.is_empty() {
            return Err(StoreError::Conflict(format!(
                "connection is used by tools: {}",
                users.join(", ")
            )));
        }
        let done = sqlx::query("DELETE FROM db_connections WHERE project_id = ? AND id = ?")
            .bind(project_id.to_string())
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(|e| match &e {
                sqlx::Error::Database(db) if db.is_foreign_key_violation() => {
                    StoreError::Conflict("connection is used by tools".into())
                }
                _ => StoreError::Db(e),
            })?;
        if done.rows_affected() == 0 {
            return Err(StoreError::NotFound("connection"));
        }
        Ok(())
    }
}
