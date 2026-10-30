//! Call log API: list calls, one call, and a summary per tool over a window.
//! Also the background task that applies each project's retention.

use std::collections::BTreeMap;
use std::time::Duration as StdDuration;

use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Duration, DurationRound, Utc};
use forge_core::{CallEntry, CallStatus};
use forge_store::{CallFilter, CallStat, Store};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::api::{ApiError, ApiPath};
use crate::AppState;

/// How often retention is applied while the server runs.
const PRUNE_EVERY: StdDuration = StdDuration::from_secs(60 * 60);

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/projects/{project_id}/calls", get(list_calls))
        .route("/projects/{project_id}/calls/summary", get(summary))
        .route("/projects/{project_id}/calls/{call_id}", get(get_call))
}

#[derive(Deserialize)]
struct ListQuery {
    limit: Option<u32>,
    tool: Option<String>,
    status: Option<String>,
    origin: Option<String>,
    since: Option<DateTime<Utc>>,
    before: Option<DateTime<Utc>>,
}

/// Newest first; page with `before` = the last entry's `ts`.
async fn list_calls(
    State(s): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<CallEntry>>, ApiError> {
    let status = match q.status.as_deref().filter(|v| !v.is_empty()) {
        None => None,
        Some(v) => Some(CallStatus::parse(v).ok_or_else(|| {
            ApiError::BadRequest(format!(
                "status must be ok, error or unknown_tool (got {v:?})"
            ))
        })?),
    };
    let nonempty = |v: Option<String>| v.filter(|v| !v.is_empty());
    let filter = CallFilter {
        tool: nonempty(q.tool),
        status,
        origin: nonempty(q.origin),
        since: q.since,
        before: q.before,
    };
    Ok(Json(
        s.store
            .list_calls(project_id, &filter, q.limit.unwrap_or(100))
            .await?,
    ))
}

async fn get_call(
    State(s): State<AppState>,
    ApiPath((project_id, call_id)): ApiPath<(Uuid, Uuid)>,
) -> Result<Json<CallEntry>, ApiError> {
    Ok(Json(s.store.get_call(project_id, call_id).await?))
}

#[derive(Deserialize)]
struct SummaryQuery {
    window: Option<String>,
}

/// Time span a summary covers, and the bucket size of its chart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Window {
    Day,
    Week,
    Month,
}

impl Window {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "24h" => Some(Window::Day),
            "7d" => Some(Window::Week),
            "30d" => Some(Window::Month),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Window::Day => "24h",
            Window::Week => "7d",
            Window::Month => "30d",
        }
    }

    /// Bucket length and bucket count.
    fn buckets(self) -> (Duration, i32) {
        match self {
            Window::Day => (Duration::hours(1), 24),
            Window::Week => (Duration::days(1), 7),
            Window::Month => (Duration::days(1), 30),
        }
    }

    /// Start of the first bucket: buckets are whole hours or UTC days, the
    /// last one containing `now`.
    pub fn start(self, now: DateTime<Utc>) -> DateTime<Utc> {
        let (len, n) = self.buckets();
        let current = now.duration_trunc(len).expect("bucket length fits");
        current - len * (n - 1)
    }
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Summary {
    pub window: &'static str,
    pub since: DateTime<Utc>,
    /// `hour` or `day`.
    pub bucket: &'static str,
    pub total: u64,
    /// Calls that didn't end `ok` (errors and unknown tools).
    pub errors: u64,
    pub p50_ms: Option<u64>,
    pub p95_ms: Option<u64>,
    /// Busiest first.
    pub tools: Vec<ToolSummary>,
    /// Every bucket in the window, oldest first, empty ones included.
    pub buckets: Vec<Bucket>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct ToolSummary {
    pub tool_name: String,
    pub calls: u64,
    pub errors: u64,
    pub p50_ms: Option<u64>,
    pub p95_ms: Option<u64>,
    pub last_call: DateTime<Utc>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Bucket {
    pub start: DateTime<Utc>,
    pub ok: u64,
    pub errors: u64,
}

async fn summary(
    State(s): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
    Query(q): Query<SummaryQuery>,
) -> Result<Json<Summary>, ApiError> {
    let window = match q.window.as_deref() {
        None => Window::Day,
        Some(w) => Window::parse(w).ok_or_else(|| {
            ApiError::BadRequest(format!("window must be 24h, 7d or 30d (got {w:?})"))
        })?,
    };
    let now = Utc::now();
    let stats = s.store.call_stats(project_id, window.start(now)).await?;
    Ok(Json(summarize(window, now, &stats)))
}

/// Nearest-rank percentile of sorted values.
fn percentile(sorted: &[u64], p: f64) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = (p * sorted.len() as f64).ceil() as usize;
    Some(sorted[rank.clamp(1, sorted.len()) - 1])
}

pub fn summarize(window: Window, now: DateTime<Utc>, stats: &[CallStat]) -> Summary {
    let since = window.start(now);
    let (len, n) = window.buckets();
    let mut buckets: Vec<Bucket> = (0..n)
        .map(|i| Bucket {
            start: since + len * i,
            ok: 0,
            errors: 0,
        })
        .collect();
    struct Acc {
        durations: Vec<u64>,
        errors: u64,
        last: DateTime<Utc>,
    }
    let mut tools: BTreeMap<&str, Acc> = BTreeMap::new();
    let mut all = Vec::with_capacity(stats.len());
    let mut errors = 0;
    for st in stats.iter().filter(|st| st.ts >= since) {
        let failed = st.status != CallStatus::Ok.as_str();
        let ms = st.duration_ms.max(0) as u64;
        all.push(ms);
        errors += u64::from(failed);
        let acc = tools.entry(&st.tool_name).or_insert(Acc {
            durations: vec![],
            errors: 0,
            last: st.ts,
        });
        acc.durations.push(ms);
        acc.errors += u64::from(failed);
        acc.last = acc.last.max(st.ts);
        let idx = ((st.ts - since).num_seconds() / len.num_seconds()) as usize;
        if let Some(b) = buckets.get_mut(idx) {
            if failed {
                b.errors += 1;
            } else {
                b.ok += 1;
            }
        }
    }
    all.sort_unstable();
    let mut tools: Vec<ToolSummary> = tools
        .into_iter()
        .map(|(name, mut acc)| {
            acc.durations.sort_unstable();
            ToolSummary {
                tool_name: name.to_owned(),
                calls: acc.durations.len() as u64,
                errors: acc.errors,
                p50_ms: percentile(&acc.durations, 0.5),
                p95_ms: percentile(&acc.durations, 0.95),
                last_call: acc.last,
            }
        })
        .collect();
    tools.sort_by(|a, b| b.calls.cmp(&a.calls).then(a.tool_name.cmp(&b.tool_name)));
    Summary {
        window: window.label(),
        since,
        bucket: if len == Duration::hours(1) {
            "hour"
        } else {
            "day"
        },
        total: all.len() as u64,
        errors,
        p50_ms: percentile(&all, 0.5),
        p95_ms: percentile(&all, 0.95),
        tools,
        buckets,
    }
}

/// Applies call-log retention now and then every hour until `stop`.
pub fn spawn_pruner(store: Store, stop: CancellationToken) {
    tokio::spawn(async move {
        loop {
            match store.prune_call_logs(Utc::now()).await {
                Ok(0) => {}
                Ok(n) => tracing::info!(removed = n, "pruned call log"),
                Err(e) => tracing::error!(error = %e, "could not prune call log"),
            }
            tokio::select! {
                () = stop.cancelled() => break,
                () = tokio::time::sleep(PRUNE_EVERY) => {}
            }
        }
    });
}

#[cfg(test)]
#[path = "../tests/unit/calls.rs"]
mod tests;
