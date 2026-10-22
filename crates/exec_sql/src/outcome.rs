//! What a tool run returns, and what it leaves for the audit log.

use forge_core::AuditOutcome;
use serde::Serialize;
use serde_json::{json, Value};

use crate::{error::SqlError, StatementClass};

/// What a query tool returned, plus what was sent (for the test console).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SqlOutcome {
    /// Statement as executed (positional placeholders).
    pub sql: String,
    /// Bound values, in bind order.
    pub params: Vec<Value>,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub truncated: bool,
    /// Columns whose values were replaced by [`MASKED`].
    pub masked_columns: Vec<String>,
    pub elapsed_ms: u64,
    /// Set for statements that change data.
    pub write: Option<WriteInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WriteInfo {
    /// `write` or `ddl`.
    pub class: String,
    pub affected_rows: u64,
    pub committed: bool,
    pub dry_run: bool,
}

impl SqlOutcome {
    /// The agent-facing result: `{columns, rows, truncated}`, plus
    /// `{affected_rows, committed, dry_run}` for writes (`rows` then holds
    /// any `RETURNING` output).
    pub fn result_json(&self) -> Value {
        let mut v =
            json!({ "columns": self.columns, "rows": self.rows, "truncated": self.truncated });
        if let Some(w) = &self.write {
            v["affected_rows"] = json!(w.affected_rows);
            v["committed"] = json!(w.committed);
            v["dry_run"] = json!(w.dry_run);
        }
        v
    }
}

/// Argument added to write-capable tools.
pub const DRY_RUN_ARG: &str = "dry_run";

/// A write attempt to record in the audit log (it reached the database).
#[derive(Debug, Clone, PartialEq)]
pub struct AuditDraft {
    pub sql: String,
    pub params: Vec<Value>,
    pub class: StatementClass,
    pub rows_affected: Option<u64>,
    pub outcome: AuditOutcome,
    pub error: Option<String>,
}

/// A tool run's result plus, for writes, what to audit.
#[derive(Debug)]
pub struct SqlRun {
    pub result: Result<SqlOutcome, SqlError>,
    pub audit: Option<AuditDraft>,
}
