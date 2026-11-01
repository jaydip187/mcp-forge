//! MCP resources (text documents) and prompts of a project.

use chrono::{DateTime, Utc};
use forge_core::{PromptArg, PromptDef, PromptInput, ResourceDef, ResourceInput};
use uuid::Uuid;

use crate::{from_json, now, on_unique, parse_id, to_json, Store, StoreError};

#[derive(sqlx::FromRow)]
struct ResourceRow {
    id: String,
    project_id: String,
    name: String,
    description: String,
    mime_type: String,
    content: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl TryFrom<ResourceRow> for ResourceDef {
    type Error = StoreError;
    fn try_from(r: ResourceRow) -> Result<Self, StoreError> {
        Ok(ResourceDef {
            id: parse_id(&r.id)?,
            project_id: parse_id(&r.project_id)?,
            name: r.name,
            description: r.description,
            mime_type: r.mime_type,
            content: r.content,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })
    }
}

#[derive(sqlx::FromRow)]
struct PromptRow {
    id: String,
    project_id: String,
    name: String,
    description: String,
    arguments: String,
    template: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl TryFrom<PromptRow> for PromptDef {
    type Error = StoreError;
    fn try_from(r: PromptRow) -> Result<Self, StoreError> {
        let arguments: Vec<PromptArg> = from_json(&r.arguments, "prompt arguments")?;
        Ok(PromptDef {
            id: parse_id(&r.id)?,
            project_id: parse_id(&r.project_id)?,
            name: r.name,
            description: r.description,
            arguments,
            template: r.template,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })
    }
}

const RESOURCE_COLUMNS: &str =
    "id, project_id, name, description, mime_type, content, created_at, updated_at";
const PROMPT_COLUMNS: &str =
    "id, project_id, name, description, arguments, template, created_at, updated_at";

fn taken<'a>(kind: &'static str, name: &'a str) -> impl FnOnce() -> String + 'a {
    move || format!("a {kind} named `{name}` already exists in this project")
}

impl Store {
    pub async fn list_resources(&self, project_id: Uuid) -> Result<Vec<ResourceDef>, StoreError> {
        self.ensure_project(project_id).await?;
        sqlx::query_as::<_, ResourceRow>(&format!(
            "SELECT {RESOURCE_COLUMNS} FROM resources WHERE project_id = ? ORDER BY name"
        ))
        .bind(project_id.to_string())
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(ResourceDef::try_from)
        .collect()
    }

    pub async fn get_resource(
        &self,
        project_id: Uuid,
        id: Uuid,
    ) -> Result<ResourceDef, StoreError> {
        sqlx::query_as::<_, ResourceRow>(&format!(
            "SELECT {RESOURCE_COLUMNS} FROM resources WHERE project_id = ? AND id = ?"
        ))
        .bind(project_id.to_string())
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .ok_or(StoreError::NotFound("resource"))?
        .try_into()
    }

    pub async fn create_resource(
        &self,
        project_id: Uuid,
        input: &ResourceInput,
    ) -> Result<ResourceDef, StoreError> {
        input.validate()?;
        self.ensure_project(project_id).await?;
        let ts = now();
        let r = ResourceDef {
            id: Uuid::new_v4(),
            project_id,
            name: input.name.clone(),
            description: input.description.clone(),
            mime_type: input.mime_type.clone(),
            content: input.content.clone(),
            created_at: ts,
            updated_at: ts,
        };
        sqlx::query(
            "INSERT INTO resources (id, project_id, name, description, mime_type, content, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(r.id.to_string())
        .bind(project_id.to_string())
        .bind(&r.name)
        .bind(&r.description)
        .bind(&r.mime_type)
        .bind(&r.content)
        .bind(r.created_at)
        .bind(r.updated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| on_unique(e, taken("resource", &r.name)))?;
        Ok(r)
    }

    pub async fn update_resource(
        &self,
        project_id: Uuid,
        id: Uuid,
        input: &ResourceInput,
    ) -> Result<ResourceDef, StoreError> {
        input.validate()?;
        let mut r = self.get_resource(project_id, id).await?;
        r.name = input.name.clone();
        r.description = input.description.clone();
        r.mime_type = input.mime_type.clone();
        r.content = input.content.clone();
        r.updated_at = now();
        sqlx::query(
            "UPDATE resources SET name = ?, description = ?, mime_type = ?, content = ?, updated_at = ? \
             WHERE project_id = ? AND id = ?",
        )
        .bind(&r.name)
        .bind(&r.description)
        .bind(&r.mime_type)
        .bind(&r.content)
        .bind(r.updated_at)
        .bind(project_id.to_string())
        .bind(id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| on_unique(e, taken("resource", &r.name)))?;
        Ok(r)
    }

    pub async fn delete_resource(&self, project_id: Uuid, id: Uuid) -> Result<(), StoreError> {
        let done = sqlx::query("DELETE FROM resources WHERE project_id = ? AND id = ?")
            .bind(project_id.to_string())
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        if done.rows_affected() == 0 {
            return Err(StoreError::NotFound("resource"));
        }
        Ok(())
    }

    pub async fn list_prompts(&self, project_id: Uuid) -> Result<Vec<PromptDef>, StoreError> {
        self.ensure_project(project_id).await?;
        sqlx::query_as::<_, PromptRow>(&format!(
            "SELECT {PROMPT_COLUMNS} FROM prompts WHERE project_id = ? ORDER BY name"
        ))
        .bind(project_id.to_string())
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(PromptDef::try_from)
        .collect()
    }

    pub async fn get_prompt(&self, project_id: Uuid, id: Uuid) -> Result<PromptDef, StoreError> {
        sqlx::query_as::<_, PromptRow>(&format!(
            "SELECT {PROMPT_COLUMNS} FROM prompts WHERE project_id = ? AND id = ?"
        ))
        .bind(project_id.to_string())
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .ok_or(StoreError::NotFound("prompt"))?
        .try_into()
    }

    pub async fn create_prompt(
        &self,
        project_id: Uuid,
        input: &PromptInput,
    ) -> Result<PromptDef, StoreError> {
        input.validate()?;
        self.ensure_project(project_id).await?;
        let ts = now();
        let p = PromptDef {
            id: Uuid::new_v4(),
            project_id,
            name: input.name.clone(),
            description: input.description.clone(),
            arguments: input.arguments.clone(),
            template: input.template.clone(),
            created_at: ts,
            updated_at: ts,
        };
        sqlx::query(
            "INSERT INTO prompts (id, project_id, name, description, arguments, template, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(p.id.to_string())
        .bind(project_id.to_string())
        .bind(&p.name)
        .bind(&p.description)
        .bind(to_json(&p.arguments))
        .bind(&p.template)
        .bind(p.created_at)
        .bind(p.updated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| on_unique(e, taken("prompt", &p.name)))?;
        Ok(p)
    }

    pub async fn update_prompt(
        &self,
        project_id: Uuid,
        id: Uuid,
        input: &PromptInput,
    ) -> Result<PromptDef, StoreError> {
        input.validate()?;
        let mut p = self.get_prompt(project_id, id).await?;
        p.name = input.name.clone();
        p.description = input.description.clone();
        p.arguments = input.arguments.clone();
        p.template = input.template.clone();
        p.updated_at = now();
        sqlx::query(
            "UPDATE prompts SET name = ?, description = ?, arguments = ?, template = ?, updated_at = ? \
             WHERE project_id = ? AND id = ?",
        )
        .bind(&p.name)
        .bind(&p.description)
        .bind(to_json(&p.arguments))
        .bind(&p.template)
        .bind(p.updated_at)
        .bind(project_id.to_string())
        .bind(id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| on_unique(e, taken("prompt", &p.name)))?;
        Ok(p)
    }

    pub async fn delete_prompt(&self, project_id: Uuid, id: Uuid) -> Result<(), StoreError> {
        let done = sqlx::query("DELETE FROM prompts WHERE project_id = ? AND id = ?")
            .bind(project_id.to_string())
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        if done.rows_affected() == 0 {
            return Err(StoreError::NotFound("prompt"));
        }
        Ok(())
    }
}
