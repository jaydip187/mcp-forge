use chrono::{DateTime, Utc};
use forge_core::{Project, ProjectInput, ProjectSettings};
use uuid::Uuid;

use crate::{from_json, now, on_unique, parse_id, to_json, Store, StoreError};

#[derive(sqlx::FromRow)]
struct ProjectRow {
    id: String,
    name: String,
    slug: String,
    settings: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl TryFrom<ProjectRow> for Project {
    type Error = StoreError;
    fn try_from(r: ProjectRow) -> Result<Self, StoreError> {
        Ok(Project {
            id: parse_id(&r.id)?,
            name: r.name,
            slug: r.slug,
            settings: from_json(&r.settings, "project settings")?,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })
    }
}

const COLUMNS: &str = "id, name, slug, settings, created_at, updated_at";

fn slug_taken(slug: &str) -> impl FnOnce() -> String + '_ {
    move || format!("a project with slug `{slug}` already exists")
}

impl Store {
    pub async fn create_project(&self, input: &ProjectInput) -> Result<Project, StoreError> {
        let (name, slug) = input.validate()?;
        let ts = now();
        let project = Project {
            id: Uuid::new_v4(),
            name,
            slug,
            settings: ProjectSettings::default(),
            created_at: ts,
            updated_at: ts,
        };
        sqlx::query(
            "INSERT INTO projects (id, name, slug, settings, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(project.id.to_string())
        .bind(&project.name)
        .bind(&project.slug)
        .bind(to_json(&project.settings))
        .bind(project.created_at)
        .bind(project.updated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| on_unique(e, slug_taken(&project.slug)))?;
        Ok(project)
    }

    pub async fn list_projects(&self) -> Result<Vec<Project>, StoreError> {
        sqlx::query_as::<_, ProjectRow>(&format!(
            "SELECT {COLUMNS} FROM projects ORDER BY name COLLATE NOCASE, slug"
        ))
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(Project::try_from)
        .collect()
    }

    pub async fn get_project(&self, id: Uuid) -> Result<Project, StoreError> {
        sqlx::query_as::<_, ProjectRow>(&format!("SELECT {COLUMNS} FROM projects WHERE id = ?"))
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?
            .ok_or(StoreError::NotFound("project"))?
            .try_into()
    }

    pub async fn get_project_by_slug(&self, slug: &str) -> Result<Project, StoreError> {
        sqlx::query_as::<_, ProjectRow>(&format!("SELECT {COLUMNS} FROM projects WHERE slug = ?"))
            .bind(slug)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(StoreError::NotFound("project"))?
            .try_into()
    }

    /// Replaces name and slug. An omitted slug keeps the current one, so
    /// renaming never silently moves the project's MCP URL.
    pub async fn update_project(
        &self,
        id: Uuid,
        input: &ProjectInput,
    ) -> Result<Project, StoreError> {
        let mut project = self.get_project(id).await?;
        let input = ProjectInput {
            name: input.name.clone(),
            slug: Some(input.slug.clone().unwrap_or(project.slug.clone())),
        };
        let (name, slug) = input.validate()?;
        project.name = name;
        project.slug = slug;
        project.updated_at = now();
        sqlx::query("UPDATE projects SET name = ?, slug = ?, updated_at = ? WHERE id = ?")
            .bind(&project.name)
            .bind(&project.slug)
            .bind(project.updated_at)
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(|e| on_unique(e, slug_taken(&project.slug)))?;
        Ok(project)
    }

    /// Replaces the project's curation and shaping settings.
    pub async fn update_project_settings(
        &self,
        id: Uuid,
        settings: &ProjectSettings,
    ) -> Result<Project, StoreError> {
        settings.validate()?;
        let mut project = self.get_project(id).await?;
        project.settings = settings.clone();
        project.updated_at = now();
        sqlx::query("UPDATE projects SET settings = ?, updated_at = ? WHERE id = ?")
            .bind(to_json(&project.settings))
            .bind(project.updated_at)
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(project)
    }

    /// Deletes the project and, by cascade, its tools and auth configs.
    pub async fn delete_project(&self, id: Uuid) -> Result<(), StoreError> {
        let done = sqlx::query("DELETE FROM projects WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        if done.rows_affected() == 0 {
            return Err(StoreError::NotFound("project"));
        }
        Ok(())
    }

    pub(crate) async fn ensure_project(&self, id: Uuid) -> Result<(), StoreError> {
        let found: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM projects WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        found.map(|_| ()).ok_or(StoreError::NotFound("project"))
    }
}
