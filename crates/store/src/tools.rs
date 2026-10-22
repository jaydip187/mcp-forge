use chrono::{DateTime, Utc};
use forge_core::{changed_fields, Overrides, Source, ToolDef, ToolInput, ValidationErrors};
use uuid::Uuid;

use crate::{from_json, now, on_unique, parse_id, to_json, Store, StoreError};

#[derive(sqlx::FromRow)]
struct ToolRow {
    id: String,
    project_id: String,
    name: String,
    description: String,
    kind: String,
    enabled: bool,
    tags: String,
    source: String,
    overrides: String,
    response_shape: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl TryFrom<ToolRow> for ToolDef {
    type Error = StoreError;
    fn try_from(r: ToolRow) -> Result<Self, StoreError> {
        Ok(ToolDef {
            id: parse_id(&r.id)?,
            project_id: parse_id(&r.project_id)?,
            name: r.name,
            description: r.description,
            kind: from_json(&r.kind, "tool kind")?,
            enabled: r.enabled,
            tags: from_json(&r.tags, "tool tags")?,
            source: from_json(&r.source, "tool source")?,
            overrides: from_json(&r.overrides, "tool overrides")?,
            response_shape: r.response_shape,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })
    }
}

const COLUMNS: &str = "id, project_id, name, description, kind, enabled, tags, source, overrides, \
                       response_shape, created_at, updated_at";

fn name_taken(name: &str) -> impl FnOnce() -> String + '_ {
    move || format!("a tool named `{name}` already exists in this project")
}

pub(crate) fn new_tool_def(project_id: Uuid, input: &ToolInput, source: Source) -> ToolDef {
    let ts = now();
    ToolDef {
        id: Uuid::new_v4(),
        project_id,
        name: input.name.clone(),
        description: input.description.clone(),
        kind: input.kind.clone(),
        enabled: input.enabled,
        tags: input.tags.clone(),
        source,
        overrides: Overrides::default(),
        response_shape: input.response_shape.clone(),
        created_at: ts,
        updated_at: ts,
    }
}

pub(crate) fn apply_input(tool: &mut ToolDef, input: &ToolInput) {
    tool.name = input.name.clone();
    tool.description = input.description.clone();
    tool.kind = input.kind.clone();
    tool.enabled = input.enabled;
    tool.tags = input.tags.clone();
    tool.response_shape = input.response_shape.clone();
    tool.updated_at = now();
}

/// Writes every mutable column of an existing tool.
pub(crate) async fn write_tool<'e, E>(exec: E, tool: &ToolDef) -> Result<(), StoreError>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    sqlx::query(
        "UPDATE tools SET name = ?, description = ?, kind = ?, auth_ref = ?, connection_ref = ?, enabled = ?, 
         tags = ?, source = ?, overrides = ?, response_shape = ?, updated_at = ? 
         WHERE project_id = ? AND id = ?",
    )
    .bind(&tool.name)
    .bind(&tool.description)
    .bind(to_json(&tool.kind))
    .bind(tool.kind.auth_ref().map(|id| id.to_string()))
    .bind(tool.kind.connection_ref().map(|id| id.to_string()))
    .bind(tool.enabled)
    .bind(to_json(&tool.tags))
    .bind(to_json(&tool.source))
    .bind(to_json(&tool.overrides))
    .bind(&tool.response_shape)
    .bind(tool.updated_at)
    .bind(tool.project_id.to_string())
    .bind(tool.id.to_string())
    .execute(exec)
    .await
    .map_err(|e| on_unique(e, name_taken(&tool.name)))?;
    Ok(())
}

pub(crate) async fn insert_tool<'e, E>(exec: E, tool: &ToolDef) -> Result<(), StoreError>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    sqlx::query(
        "INSERT INTO tools (id, project_id, name, description, kind, auth_ref, connection_ref,          enabled, tags, source, overrides, response_shape, created_at, updated_at)          VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(tool.id.to_string())
    .bind(tool.project_id.to_string())
    .bind(&tool.name)
    .bind(&tool.description)
    .bind(to_json(&tool.kind))
    .bind(tool.kind.auth_ref().map(|id| id.to_string()))
    .bind(tool.kind.connection_ref().map(|id| id.to_string()))
    .bind(tool.enabled)
    .bind(to_json(&tool.tags))
    .bind(to_json(&tool.source))
    .bind(to_json(&tool.overrides))
    .bind(&tool.response_shape)
    .bind(tool.created_at)
    .bind(tool.updated_at)
    .execute(exec)
    .await
    .map_err(|e| on_unique(e, name_taken(&tool.name)))?;
    Ok(())
}

impl Store {
    /// Creates a manual tool. Importers will set other sources (Stage 4+).
    pub async fn create_tool(
        &self,
        project_id: Uuid,
        input: &ToolInput,
    ) -> Result<ToolDef, StoreError> {
        input.validate()?;
        self.ensure_project(project_id).await?;
        self.check_auth_ref(project_id, input).await?;
        let tool = new_tool_def(project_id, input, Source::Manual);
        insert_tool(&self.pool, &tool).await?;
        Ok(tool)
    }

    pub async fn list_tools(&self, project_id: Uuid) -> Result<Vec<ToolDef>, StoreError> {
        self.ensure_project(project_id).await?;
        sqlx::query_as::<_, ToolRow>(&format!(
            "SELECT {COLUMNS} FROM tools WHERE project_id = ? ORDER BY name"
        ))
        .bind(project_id.to_string())
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(ToolDef::try_from)
        .collect()
    }

    pub async fn get_tool(&self, project_id: Uuid, id: Uuid) -> Result<ToolDef, StoreError> {
        sqlx::query_as::<_, ToolRow>(&format!(
            "SELECT {COLUMNS} FROM tools WHERE project_id = ? AND id = ?"
        ))
        .bind(project_id.to_string())
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .ok_or(StoreError::NotFound("tool"))?
        .try_into()
    }

    pub async fn get_tool_by_name(
        &self,
        project_id: Uuid,
        name: &str,
    ) -> Result<ToolDef, StoreError> {
        sqlx::query_as::<_, ToolRow>(&format!(
            "SELECT {COLUMNS} FROM tools WHERE project_id = ? AND name = ?"
        ))
        .bind(project_id.to_string())
        .bind(name)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(StoreError::NotFound("tool"))?
        .try_into()
    }

    /// Cheap value that changes whenever the project's tool set changes
    /// (add, delete, edit, enable/disable). Used to detect out-of-process edits.
    pub async fn tools_fingerprint(&self, project_id: Uuid) -> Result<String, StoreError> {
        let (fp,): (String,) = sqlx::query_as(
            "SELECT COALESCE(GROUP_CONCAT(id || ':' || updated_at, ','), '')              FROM (SELECT id, updated_at FROM tools WHERE project_id = ? ORDER BY id)",
        )
        .bind(project_id.to_string())
        .fetch_one(&self.pool)
        .await?;
        Ok(fp)
    }

    /// Replaces the user-editable fields; `source` and `created_at` are kept.
    /// On an imported tool, every field that changes is recorded in
    /// `overrides` so a later re-import keeps the user's version.
    pub async fn update_tool(
        &self,
        project_id: Uuid,
        id: Uuid,
        input: &ToolInput,
    ) -> Result<ToolDef, StoreError> {
        input.validate()?;
        let mut tool = self.get_tool(project_id, id).await?;
        self.check_auth_ref(project_id, input).await?;
        if tool.source != Source::Manual && tool.kind.type_name() != input.kind.type_name() {
            return Err(ValidationErrors::single(
                "kind.type",
                "an imported tool can't change its kind",
            )
            .into());
        }
        if tool.source != Source::Manual {
            for f in changed_fields(&tool.input(), input) {
                tool.overrides.fields.insert(f.to_owned());
            }
        }
        apply_input(&mut tool, input);
        write_tool(&self.pool, &tool).await?;
        Ok(tool)
    }

    /// Stops keeping the user's version of `fields` (all fields if `None`), so
    /// the next re-import takes the spec's values. The current values stay.
    pub async fn reset_overrides(
        &self,
        project_id: Uuid,
        id: Uuid,
        fields: Option<&[String]>,
    ) -> Result<ToolDef, StoreError> {
        let mut tool = self.get_tool(project_id, id).await?;
        match fields {
            None => tool.overrides.fields.clear(),
            Some(fs) => {
                for f in fs {
                    tool.overrides.fields.remove(f);
                }
            }
        }
        tool.updated_at = now();
        write_tool(&self.pool, &tool).await?;
        Ok(tool)
    }

    pub async fn delete_tool(&self, project_id: Uuid, id: Uuid) -> Result<(), StoreError> {
        let done = sqlx::query("DELETE FROM tools WHERE project_id = ? AND id = ?")
            .bind(project_id.to_string())
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        if done.rows_affected() == 0 {
            return Err(StoreError::NotFound("tool"));
        }
        Ok(())
    }

    /// A tool may only reference an auth config or DB connection from its
    /// own project.
    pub(crate) async fn check_auth_ref(
        &self,
        project_id: Uuid,
        input: &ToolInput,
    ) -> Result<(), StoreError> {
        if let Some(conn_id) = input.kind.connection_ref() {
            match self.get_connection(project_id, conn_id).await {
                Ok(_) => {}
                Err(StoreError::NotFound(_)) => {
                    return Err(ValidationErrors::single(
                        "kind.connection_id",
                        format!("no connection {conn_id} in this project"),
                    )
                    .into())
                }
                Err(e) => return Err(e),
            }
        }
        let Some(auth_id) = input.kind.auth_ref() else {
            return Ok(());
        };
        match self.get_auth_config(project_id, auth_id).await {
            Ok(_) => Ok(()),
            Err(StoreError::NotFound(_)) => Err(ValidationErrors::single(
                "kind.auth_ref",
                format!("no auth config {auth_id} in this project"),
            )
            .into()),
            Err(e) => Err(e),
        }
    }
}
