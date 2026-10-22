use chrono::{DateTime, Utc};
use forge_core::{Source, ToolDef, ToolInput, ValidationErrors};
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crate::tools::{apply_input, insert_tool, new_tool_def, write_tool};
use crate::{from_json, now, parse_id, to_json, Store, StoreError};

/// An API description to store alongside the tools imported from it.
#[derive(Debug, Clone)]
pub struct NewSpec {
    pub kind: String,
    pub title: String,
    pub version: String,
    pub source_url: Option<String>,
    pub filename: Option<String>,
    pub raw: String,
    /// Choices made at import (base URL, auth config), reused on re-import.
    pub options: Value,
}

/// A stored spec, without its (possibly large) raw text.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SpecRecord {
    pub id: Uuid,
    pub project_id: Uuid,
    pub kind: String,
    pub title: String,
    pub version: String,
    pub source_url: Option<String>,
    pub filename: Option<String>,
    pub options: Value,
    pub created_at: DateTime<Utc>,
    /// Last re-import, if any.
    pub updated_at: Option<DateTime<Utc>>,
}

/// What a re-import changed.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ReimportResult {
    pub updated: Vec<ToolDef>,
    pub created: Vec<ToolDef>,
    pub deleted: Vec<Uuid>,
}

#[derive(sqlx::FromRow)]
struct SpecRow {
    id: String,
    project_id: String,
    kind: String,
    title: String,
    version: String,
    source_url: Option<String>,
    filename: Option<String>,
    options: String,
    created_at: DateTime<Utc>,
    updated_at: Option<DateTime<Utc>>,
}

impl TryFrom<SpecRow> for SpecRecord {
    type Error = StoreError;
    fn try_from(r: SpecRow) -> Result<Self, StoreError> {
        Ok(SpecRecord {
            id: parse_id(&r.id)?,
            project_id: parse_id(&r.project_id)?,
            kind: r.kind,
            title: r.title,
            version: r.version,
            source_url: r.source_url,
            filename: r.filename,
            options: from_json(&r.options, "spec options")?,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })
    }
}

const COLUMNS: &str =
    "id, project_id, kind, title, version, source_url, filename, options, created_at, updated_at";

fn source_for(kind: &str, spec_id: Uuid, op_id: String) -> Source {
    match kind {
        "swagger2" => Source::Swagger { spec_id, op_id },
        _ => Source::OpenApi { spec_id, op_id },
    }
}

impl Store {
    /// Stores `spec` and creates one tool per entry, all or nothing. Each
    /// tool's `Source` gets the new spec's id. Names must already be unique.
    pub async fn import_tools(
        &self,
        project_id: Uuid,
        spec: &NewSpec,
        tools: &[(ToolInput, String)],
    ) -> Result<(SpecRecord, Vec<ToolDef>), StoreError> {
        self.ensure_project(project_id).await?;
        self.check_tools(project_id, tools.iter().map(|(t, _)| t), "tools")
            .await?;

        let record = SpecRecord {
            id: Uuid::new_v4(),
            project_id,
            kind: spec.kind.clone(),
            title: spec.title.clone(),
            version: spec.version.clone(),
            source_url: spec.source_url.clone(),
            filename: spec.filename.clone(),
            options: spec.options.clone(),
            created_at: now(),
            updated_at: None,
        };
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO specs (id, project_id, kind, title, version, source_url, filename, raw, options, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(record.id.to_string())
        .bind(project_id.to_string())
        .bind(&record.kind)
        .bind(&record.title)
        .bind(&record.version)
        .bind(&record.source_url)
        .bind(&record.filename)
        .bind(&spec.raw)
        .bind(to_json(&record.options))
        .bind(record.created_at)
        .execute(&mut *tx)
        .await?;

        let mut created = Vec::with_capacity(tools.len());
        for (input, op_id) in tools {
            let tool = new_tool_def(
                project_id,
                input,
                source_for(&spec.kind, record.id, op_id.clone()),
            );
            insert_tool(&mut *tx, &tool).await?;
            created.push(tool);
        }
        tx.commit().await?;
        Ok((record, created))
    }

    /// Applies a re-import in one transaction: replaces the stored spec, then
    /// deletes, updates (overrides kept as they are) and creates tools.
    pub async fn apply_reimport(
        &self,
        project_id: Uuid,
        spec_id: Uuid,
        spec: &NewSpec,
        updates: &[(Uuid, ToolInput, String)],
        inserts: &[(ToolInput, String)],
        deletes: &[Uuid],
    ) -> Result<(SpecRecord, ReimportResult), StoreError> {
        let mut record = self.get_spec(project_id, spec_id).await?;
        self.check_tools(project_id, updates.iter().map(|(_, t, _)| t), "updates")
            .await?;
        self.check_tools(project_id, inserts.iter().map(|(t, _)| t), "inserts")
            .await?;

        let mut current = Vec::with_capacity(updates.len());
        for (id, _, _) in updates {
            let t = self.get_tool(project_id, *id).await?;
            if t.source.spec_id() != Some(spec_id) {
                return Err(StoreError::Conflict(format!(
                    "tool `{}` doesn't come from this spec",
                    t.name
                )));
            }
            current.push(t);
        }

        record.kind = spec.kind.clone();
        record.title = spec.title.clone();
        record.version = spec.version.clone();
        record.source_url = spec.source_url.clone();
        record.filename = spec.filename.clone();
        record.options = spec.options.clone();
        record.updated_at = Some(now());

        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "UPDATE specs SET kind = ?, title = ?, version = ?, source_url = ?, filename = ?, raw = ?, \
             options = ?, updated_at = ? WHERE project_id = ? AND id = ?",
        )
        .bind(&record.kind)
        .bind(&record.title)
        .bind(&record.version)
        .bind(&record.source_url)
        .bind(&record.filename)
        .bind(&spec.raw)
        .bind(to_json(&record.options))
        .bind(record.updated_at)
        .bind(project_id.to_string())
        .bind(spec_id.to_string())
        .execute(&mut *tx)
        .await?;

        let mut result = ReimportResult::default();
        for id in deletes {
            let done = sqlx::query(
                "DELETE FROM tools WHERE project_id = ? AND id = ? AND json_extract(source, '$.spec_id') = ?",
            )
            .bind(project_id.to_string())
            .bind(id.to_string())
            .bind(spec_id.to_string())
            .execute(&mut *tx)
            .await?;
            if done.rows_affected() == 1 {
                result.deleted.push(*id);
            }
        }
        for (mut tool, (_, input, op_id)) in current.into_iter().zip(updates) {
            apply_input(&mut tool, input);
            tool.source = source_for(&spec.kind, spec_id, op_id.clone());
            write_tool(&mut *tx, &tool).await?;
            result.updated.push(tool);
        }
        for (input, op_id) in inserts {
            let tool = new_tool_def(
                project_id,
                input,
                source_for(&spec.kind, spec_id, op_id.clone()),
            );
            insert_tool(&mut *tx, &tool).await?;
            result.created.push(tool);
        }
        tx.commit().await?;
        Ok((record, result))
    }

    pub async fn list_specs(&self, project_id: Uuid) -> Result<Vec<SpecRecord>, StoreError> {
        self.ensure_project(project_id).await?;
        sqlx::query_as::<_, SpecRow>(&format!(
            "SELECT {COLUMNS} FROM specs WHERE project_id = ? ORDER BY created_at DESC"
        ))
        .bind(project_id.to_string())
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(SpecRecord::try_from)
        .collect()
    }

    pub async fn get_spec(&self, project_id: Uuid, id: Uuid) -> Result<SpecRecord, StoreError> {
        sqlx::query_as::<_, SpecRow>(&format!(
            "SELECT {COLUMNS} FROM specs WHERE project_id = ? AND id = ?"
        ))
        .bind(project_id.to_string())
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .ok_or(StoreError::NotFound("spec"))?
        .try_into()
    }

    /// The raw text of a stored spec.
    pub async fn get_spec_raw(&self, project_id: Uuid, id: Uuid) -> Result<String, StoreError> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT raw FROM specs WHERE project_id = ? AND id = ?")
                .bind(project_id.to_string())
                .bind(id.to_string())
                .fetch_optional(&self.pool)
                .await?;
        row.map(|(r,)| r).ok_or(StoreError::NotFound("spec"))
    }

    /// Tools imported from one spec.
    pub async fn tools_for_spec(
        &self,
        project_id: Uuid,
        spec_id: Uuid,
    ) -> Result<Vec<ToolDef>, StoreError> {
        Ok(self
            .list_tools(project_id)
            .await?
            .into_iter()
            .filter(|t| t.source.spec_id() == Some(spec_id))
            .collect())
    }

    /// Names of all tools in the project, for picking non-clashing import names.
    pub async fn tool_names(&self, project_id: Uuid) -> Result<Vec<String>, StoreError> {
        let rows: Vec<(String,)> = sqlx::query_as("SELECT name FROM tools WHERE project_id = ?")
            .bind(project_id.to_string())
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.into_iter().map(|(n,)| n).collect())
    }

    async fn check_tools<'a>(
        &self,
        project_id: Uuid,
        tools: impl Iterator<Item = &'a ToolInput>,
        prefix: &str,
    ) -> Result<(), StoreError> {
        for (i, input) in tools.enumerate() {
            input
                .validate()
                .map_err(|e| prefix_errors(e, &format!("{prefix}[{i}]")))?;
            self.check_auth_ref(project_id, input).await?;
        }
        Ok(())
    }
}

fn prefix_errors(mut e: ValidationErrors, prefix: &str) -> StoreError {
    for f in &mut e.errors {
        f.field = format!("{prefix}.{}", f.field);
    }
    StoreError::Validation(e)
}
