use chrono::{DateTime, Duration, Utc};
use forge_core::{CallEntry, CallStatus, MAX_CALLS_PER_PROJECT};
use sqlx::QueryBuilder;
use uuid::Uuid;

use crate::{from_json, parse_id, to_json, Store, StoreError};

/// Most calls returned by one listing.
pub const MAX_CALL_PAGE: u32 = 500;

/// Narrows a call listing. Empty fields match everything.
#[derive(Debug, Clone, Default)]
pub struct CallFilter {
    pub tool: Option<String>,
    pub status: Option<CallStatus>,
    pub origin: Option<String>,
    /// Only calls at or after this time.
    pub since: Option<DateTime<Utc>>,
    /// Only calls strictly before this time (paging backwards).
    pub before: Option<DateTime<Utc>>,
}

/// The fields the summary needs, one per call.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct CallStat {
    pub ts: DateTime<Utc>,
    pub tool_name: String,
    pub status: String,
    pub duration_ms: i64,
}

#[derive(sqlx::FromRow)]
struct CallRow {
    id: String,
    project_id: String,
    ts: DateTime<Utc>,
    tool_id: Option<String>,
    tool_name: String,
    origin: String,
    client: Option<String>,
    arguments: String,
    status: String,
    duration_ms: i64,
    response_chars: i64,
    upstream_status: Option<i64>,
    error: Option<String>,
}

impl TryFrom<CallRow> for CallEntry {
    type Error = StoreError;
    fn try_from(r: CallRow) -> Result<Self, StoreError> {
        Ok(CallEntry {
            id: parse_id(&r.id)?,
            project_id: parse_id(&r.project_id)?,
            ts: r.ts,
            tool_id: r.tool_id.as_deref().map(parse_id).transpose()?,
            tool_name: r.tool_name,
            origin: r.origin,
            client: r.client,
            arguments: from_json(&r.arguments, "call arguments")?,
            status: CallStatus::parse(&r.status)
                .ok_or_else(|| StoreError::Corrupt(format!("bad call status {:?}", r.status)))?,
            duration_ms: r.duration_ms.max(0) as u64,
            response_chars: r.response_chars.max(0) as u64,
            upstream_status: r.upstream_status.and_then(|s| u16::try_from(s).ok()),
            error: r.error,
        })
    }
}

const COLUMNS: &str = "id, project_id, ts, tool_id, tool_name, origin, client, arguments, status, \
                       duration_ms, response_chars, upstream_status, error";

impl Store {
    pub async fn append_call(&self, e: &CallEntry) -> Result<(), StoreError> {
        sqlx::query(&format!(
            "INSERT INTO call_log ({COLUMNS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        ))
        .bind(e.id.to_string())
        .bind(e.project_id.to_string())
        .bind(e.ts)
        .bind(e.tool_id.map(|i| i.to_string()))
        .bind(&e.tool_name)
        .bind(&e.origin)
        .bind(&e.client)
        .bind(to_json(&e.arguments))
        .bind(e.status.as_str())
        .bind(i64::try_from(e.duration_ms).unwrap_or(i64::MAX))
        .bind(i64::try_from(e.response_chars).unwrap_or(i64::MAX))
        .bind(e.upstream_status.map(i64::from))
        .bind(&e.error)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Newest first, at most `limit` (capped at [`MAX_CALL_PAGE`]).
    pub async fn list_calls(
        &self,
        project_id: Uuid,
        filter: &CallFilter,
        limit: u32,
    ) -> Result<Vec<CallEntry>, StoreError> {
        self.ensure_project(project_id).await?;
        let mut q = QueryBuilder::new(format!(
            "SELECT {COLUMNS} FROM call_log WHERE project_id = "
        ));
        q.push_bind(project_id.to_string());
        if let Some(t) = &filter.tool {
            q.push(" AND tool_name = ").push_bind(t.clone());
        }
        if let Some(s) = filter.status {
            q.push(" AND status = ").push_bind(s.as_str());
        }
        if let Some(o) = &filter.origin {
            q.push(" AND origin = ").push_bind(o.clone());
        }
        if let Some(t) = filter.since {
            q.push(" AND ts >= ").push_bind(t);
        }
        if let Some(t) = filter.before {
            q.push(" AND ts < ").push_bind(t);
        }
        q.push(" ORDER BY ts DESC, id LIMIT ")
            .push_bind(i64::from(limit.clamp(1, MAX_CALL_PAGE)));
        q.build_query_as::<CallRow>()
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(CallEntry::try_from)
            .collect()
    }

    pub async fn get_call(&self, project_id: Uuid, id: Uuid) -> Result<CallEntry, StoreError> {
        sqlx::query_as::<_, CallRow>(&format!(
            "SELECT {COLUMNS} FROM call_log WHERE project_id = ? AND id = ?"
        ))
        .bind(project_id.to_string())
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .ok_or(StoreError::NotFound("call"))?
        .try_into()
    }

    /// Calls since `since`, oldest first, for the summary.
    pub async fn call_stats(
        &self,
        project_id: Uuid,
        since: DateTime<Utc>,
    ) -> Result<Vec<CallStat>, StoreError> {
        self.ensure_project(project_id).await?;
        Ok(sqlx::query_as::<_, CallStat>(
            "SELECT ts, tool_name, status, duration_ms FROM call_log \
             WHERE project_id = ? AND ts >= ? ORDER BY ts LIMIT ?",
        )
        .bind(project_id.to_string())
        .bind(since)
        .bind(i64::from(MAX_CALLS_PER_PROJECT))
        .fetch_all(&self.pool)
        .await?)
    }

    /// Drops a project's calls older than `before`, then the oldest beyond
    /// `keep` entries. Returns how many went.
    pub async fn prune_calls(
        &self,
        project_id: Uuid,
        before: DateTime<Utc>,
        keep: u32,
    ) -> Result<u64, StoreError> {
        let pid = project_id.to_string();
        let old = sqlx::query("DELETE FROM call_log WHERE project_id = ? AND ts < ?")
            .bind(&pid)
            .bind(before)
            .execute(&self.pool)
            .await?
            .rows_affected();
        let over = sqlx::query(
            "DELETE FROM call_log WHERE project_id = ? AND id NOT IN \
             (SELECT id FROM call_log WHERE project_id = ? ORDER BY ts DESC, id LIMIT ?)",
        )
        .bind(&pid)
        .bind(&pid)
        .bind(i64::from(keep))
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok(old + over)
    }

    /// Applies every project's `call_log_days` (0 clears its log) and the
    /// per-project cap.
    pub async fn prune_call_logs(&self, now: DateTime<Utc>) -> Result<u64, StoreError> {
        let mut gone = 0;
        for p in self.list_projects().await? {
            let before = now - Duration::days(i64::from(p.settings.call_log_days));
            gone += self
                .prune_calls(p.id, before, MAX_CALLS_PER_PROJECT)
                .await?;
        }
        Ok(gone)
    }
}
