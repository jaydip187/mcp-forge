//! Calls waiting for a person's approval.
//!
//! A pending call can be answered from two places: the MCP client (form
//! elicitation, when it supports it) and the web UI's Approvals page. Both go
//! through [`Approvals::decide`], so whichever answers first wins.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use forge_core::ApprovalDecision;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::oneshot;
use uuid::Uuid;

/// A call waiting for approval, as the web UI lists it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PendingApproval {
    pub id: Uuid,
    pub project_id: Uuid,
    pub tool_id: Uuid,
    pub tool_name: String,
    /// MCP client as `name version`, when known.
    pub client: Option<String>,
    /// Arguments as the call log records them (secret-looking values redacted).
    pub arguments: Value,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    /// Whether the client was also asked (form elicitation).
    pub asked_client: bool,
}

/// The answer to a pending call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub decision: ApprovalDecision,
    pub by: Option<String>,
}

struct Entry {
    pending: PendingApproval,
    reply: oneshot::Sender<Decision>,
}

/// Registry of calls waiting for approval. Cheap to clone; clones share state.
#[derive(Clone)]
pub struct Approvals {
    inner: Arc<Mutex<HashMap<Uuid, Entry>>>,
    /// Whether a web UI can answer (false in stdio mode).
    has_ui: bool,
}

impl Default for Approvals {
    fn default() -> Self {
        Self {
            inner: Arc::default(),
            has_ui: true,
        }
    }
}

impl Approvals {
    /// A registry nobody can answer from a web UI (stdio mode).
    pub fn without_ui() -> Self {
        Self {
            has_ui: false,
            ..Self::default()
        }
    }

    pub fn has_ui(&self) -> bool {
        self.has_ui
    }

    /// Registers a pending call; the receiver gets the decision.
    pub fn open(&self, pending: PendingApproval) -> oneshot::Receiver<Decision> {
        let (reply, rx) = oneshot::channel();
        self.lock().insert(pending.id, Entry { pending, reply });
        rx
    }

    /// Pending calls, oldest first.
    pub fn list(&self) -> Vec<PendingApproval> {
        let mut all: Vec<_> = self.lock().values().map(|e| e.pending.clone()).collect();
        all.sort_by_key(|p| p.created_at);
        all
    }

    pub fn get(&self, id: Uuid) -> Option<PendingApproval> {
        self.lock().get(&id).map(|e| e.pending.clone())
    }

    /// Answers a pending call. `false` if it was no longer pending.
    pub fn decide(&self, id: Uuid, decision: ApprovalDecision, by: Option<String>) -> bool {
        let Some(entry) = self.lock().remove(&id) else {
            return false;
        };
        // The waiter may have gone away (client disconnected); that's fine.
        let _ = entry.reply.send(Decision { decision, by });
        true
    }

    /// Cancels every pending call (server shutting down).
    pub fn cancel_all(&self) {
        let drained: Vec<Entry> = self.lock().drain().map(|(_, e)| e).collect();
        for e in drained {
            let _ = e.reply.send(Decision {
                decision: ApprovalDecision::Cancelled,
                by: None,
            });
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<Uuid, Entry>> {
        self.inner.lock().expect("approvals lock poisoned")
    }
}

/// What the agent is told when a call wasn't approved.
pub fn not_approved_text(d: &Decision, timeout_secs: u32) -> String {
    match d.decision {
        ApprovalDecision::Approved => unreachable!("approved calls run"),
        ApprovalDecision::Denied => format!(
            "Not run: this tool needs a person's approval and {} denied this call. \
             Don't retry it unless the user asks you to.",
            d.by.as_deref().unwrap_or("someone")
        ),
        ApprovalDecision::TimedOut => format!(
            "Not run: this tool needs a person's approval and nobody answered within \
             {timeout_secs} seconds. Ask the user to approve it (in mcp-forge under \
             Approvals, or in this client) and then try again."
        ),
        ApprovalDecision::Cancelled => "Not run: the approval request was cancelled \
             (the server stopped or the connection closed)."
            .into(),
    }
}

/// Shown when nobody could ever answer: stdio mode and no elicitation.
pub const NO_WAY_TO_APPROVE: &str = "Not run: this tool needs a person's approval, but this \
     client can't show approval prompts and stdio mode has no web UI. Connect over HTTP \
     (mcp-forge serve) to approve calls in the browser, or turn off approval for this tool.";

#[cfg(test)]
#[path = "../tests/unit/approvals.rs"]
mod tests;
