//! Results of NoSQL tools.

use forge_exec_sql::{AuditDraft, SqlError, WriteInfo};
use serde::Serialize;
use serde_json::{json, Value};

/// What a NoSQL tool returned.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct NosqlOutcome {
    /// The operation as sent: `{operation, target, body}` (templates filled).
    pub request: Value,
    pub documents: Vec<Value>,
    /// More documents existed than the row limit.
    pub truncated: bool,
    /// Counts, totals or aggregations, when the engine returns them.
    pub summary: Option<Value>,
    /// Fields whose values were replaced by `[masked]`.
    pub masked_fields: Vec<String>,
    pub elapsed_ms: u64,
    pub write: Option<WriteInfo>,
}

impl NosqlOutcome {
    /// The agent-facing result: `{documents, truncated}`, the summary's
    /// fields, and `{affected, committed, dry_run}` for writes.
    pub fn result_json(&self) -> Value {
        let mut v = json!({ "documents": self.documents, "truncated": self.truncated });
        if let Some(Value::Object(s)) = &self.summary {
            for (k, x) in s {
                v[k] = x.clone();
            }
        }
        if let Some(w) = &self.write {
            v["affected"] = json!(w.affected_rows);
            v["committed"] = json!(w.committed);
            v["dry_run"] = json!(w.dry_run);
        }
        v
    }
}

/// A tool run's result plus, for writes, what to audit.
#[derive(Debug)]
pub struct NosqlRun {
    pub result: Result<NosqlOutcome, SqlError>,
    pub audit: Option<AuditDraft>,
}
