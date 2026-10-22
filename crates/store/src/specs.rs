use chrono::{DateTime, Utc};
use forge_core::{Source, ToolDef, ToolInput, ValidationErrors};
use serde::Serialize;
use uuid::Uuid;

use crate::tools::{insert_tool, new_tool_def};
use crate::{now, parse_id, Store, StoreError};

/// An API description to store alongside the tools imported from it.
#[derive(Debug, Clone)]
pub struct NewSpec {
    pub kind: String,
    pub title: String,
    pub version: String,
    pub source_url: Option<String>,
    pub filename: Option<String>,
    pub raw: String,
}

/// A stored spec, without its (possibly large) raw text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SpecRecord {
    pub id: Uuid,
    pub project_id: Uuid,
    pub kind: String,
    pub title: String,
    pub version: String,
    pub source_url: Option<String>,
    pub filename: Option<String>,
    pub created_at: DateTime<Utc>,
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
    created_at: DateTime<Utc>,
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
            created_at: r.created_at,
        })
    }
}

const COLUMNS: &str = "id, project_id, kind, title, version, source_url, filename, created_at";

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
        for (i, (input, _)) in tools.iter().enumerate() {
            input
                .validate()
                .map_err(|e| prefix_errors(e, &format!("tools[{i}]")))?;
            self.check_auth_ref(project_id, input).await?;
        }

        let record = SpecRecord {
            id: Uuid::new_v4(),
            project_id,
            kind: spec.kind.clone(),
            title: spec.title.clone(),
            version: spec.version.clone(),
            source_url: spec.source_url.clone(),
            filename: spec.filename.clone(),
            created_at: now(),
        };
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO specs (id, project_id, kind, title, version, source_url, filename, raw, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(record.id.to_string())
        .bind(project_id.to_string())
        .bind(&record.kind)
        .bind(&record.title)
        .bind(&record.version)
        .bind(&record.source_url)
        .bind(&record.filename)
        .bind(&spec.raw)
        .bind(record.created_at)
        .execute(&mut *tx)
        .await?;

        let mut created = Vec::with_capacity(tools.len());
        for (input, op_id) in tools {
            let source = match spec.kind.as_str() {
                "swagger2" => Source::Swagger {
                    spec_id: record.id,
                    op_id: op_id.clone(),
                },
                _ => Source::OpenApi {
                    spec_id: record.id,
                    op_id: op_id.clone(),
                },
            };
            let tool = new_tool_def(project_id, input, source);
            insert_tool(&mut *tx, &tool).await?;
            created.push(tool);
        }
        tx.commit().await?;
        Ok((record, created))
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

    /// Names of all tools in the project, for picking non-clashing import names.
    pub async fn tool_names(&self, project_id: Uuid) -> Result<Vec<String>, StoreError> {
        let rows: Vec<(String,)> = sqlx::query_as("SELECT name FROM tools WHERE project_id = ?")
            .bind(project_id.to_string())
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.into_iter().map(|(n,)| n).collect())
    }
}

fn prefix_errors(mut e: ValidationErrors, prefix: &str) -> StoreError {
    for f in &mut e.errors {
        f.field = format!("{prefix}.{}", f.field);
    }
    StoreError::Validation(e)
}
