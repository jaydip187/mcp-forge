//! Audit log of SQL statements that change data.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// How a write ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditOutcome {
    Committed,
    /// Rolled back because it changed more rows than allowed.
    RolledBack,
    /// Rolled back on purpose (`dry_run`).
    DryRun,
    /// The database returned an error or the statement timed out.
    Failed,
}

impl AuditOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            AuditOutcome::Committed => "committed",
            AuditOutcome::RolledBack => "rolled_back",
            AuditOutcome::DryRun => "dry_run",
            AuditOutcome::Failed => "failed",
        }
    }
}

/// One write attempt that reached the database.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: Uuid,
    pub project_id: Uuid,
    pub ts: DateTime<Utc>,
    /// Kept as text: the tool or connection may be deleted later.
    pub tool_id: Option<Uuid>,
    pub tool_name: String,
    pub connection_id: Option<Uuid>,
    pub connection_name: String,
    /// `mcp` (an agent) or `console` (the UI test console).
    pub origin: String,
    /// `write` or `ddl`.
    pub statement_class: String,
    /// The statement as sent (positional placeholders).
    pub sql: String,
    pub params: Vec<Value>,
    pub rows_affected: Option<u64>,
    pub outcome: AuditOutcome,
    pub error: Option<String>,
}
