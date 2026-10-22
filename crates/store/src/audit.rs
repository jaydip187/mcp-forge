use chrono::{DateTime, Utc};
use forge_core::AuditEntry;
use uuid::Uuid;

use crate::{from_json, parse_id, to_json, Store, StoreError};

#[derive(sqlx::FromRow)]
struct AuditRow {
    id: String,
    project_id: String,
    ts: DateTime<Utc>,
    tool_id: Option<String>,
    tool_name: String,
    connection_id: Option<String>,
    connection_name: String,
    origin: String,
    statement_class: String,
    sql: String,
    params: String,
    rows_affected: Option<i64>,
    outcome: String,
    error: Option<String>,
}

impl TryFrom<AuditRow> for AuditEntry {
    type Error = StoreError;
    fn try_from(r: AuditRow) -> Result<Self, StoreError> {
        let opt_id = |s: Option<String>| s.as_deref().map(parse_id).transpose();
        Ok(AuditEntry {
            id: parse_id(&r.id)?,
            project_id: parse_id(&r.project_id)?,
            ts: r.ts,
            tool_id: opt_id(r.tool_id)?,
            tool_name: r.tool_name,
            connection_id: opt_id(r.connection_id)?,
            connection_name: r.connection_name,
            origin: r.origin,
            statement_class: r.statement_class,
            sql: r.sql,
            params: from_json(&r.params, "audit params")?,
            rows_affected: r.rows_affected.map(|n| n.max(0) as u64),
            outcome: from_json(&format!("\"{}\"", r.outcome), "audit outcome")?,
            error: r.error,
        })
    }
}

/// Most entries returned by one listing.
pub const MAX_AUDIT_PAGE: u32 = 500;

impl Store {
    pub async fn append_audit(&self, e: &AuditEntry) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO audit_log (id, project_id, ts, tool_id, tool_name, connection_id, connection_name, \
             origin, statement_class, sql, params, rows_affected, outcome, error) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(e.id.to_string())
        .bind(e.project_id.to_string())
        .bind(e.ts)
        .bind(e.tool_id.map(|i| i.to_string()))
        .bind(&e.tool_name)
        .bind(e.connection_id.map(|i| i.to_string()))
        .bind(&e.connection_name)
        .bind(&e.origin)
        .bind(&e.statement_class)
        .bind(&e.sql)
        .bind(to_json(&e.params))
        .bind(e.rows_affected.map(|n| n as i64))
        .bind(e.outcome.as_str())
        .bind(&e.error)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Newest first; `before` pages backwards by timestamp.
    pub async fn list_audit(
        &self,
        project_id: Uuid,
        limit: u32,
        before: Option<DateTime<Utc>>,
    ) -> Result<Vec<AuditEntry>, StoreError> {
        self.ensure_project(project_id).await?;
        let limit = limit.clamp(1, MAX_AUDIT_PAGE);
        sqlx::query_as::<_, AuditRow>(
            "SELECT id, project_id, ts, tool_id, tool_name, connection_id, connection_name, origin, \
             statement_class, sql, params, rows_affected, outcome, error FROM audit_log \
             WHERE project_id = ? AND (? IS NULL OR ts < ?) ORDER BY ts DESC, id LIMIT ?",
        )
        .bind(project_id.to_string())
        .bind(before)
        .bind(before)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(AuditEntry::try_from)
        .collect()
    }
}
