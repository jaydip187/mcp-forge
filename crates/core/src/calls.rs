//! Log of tool calls, from agents (MCP) and the UI test console.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

/// Recorded arguments are cut to this many characters of JSON.
pub const MAX_LOGGED_ARGS_CHARS: usize = 4_000;
/// Recorded error text is cut to this many characters.
pub const MAX_LOGGED_ERROR_CHARS: usize = 1_000;
/// Oldest calls beyond this many per project are pruned.
pub const MAX_CALLS_PER_PROJECT: u32 = 100_000;
/// Shown instead of argument values whose names look secret.
pub const REDACTED: &str = "[redacted]";

/// How a call ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallStatus {
    /// The agent got a normal result.
    Ok,
    /// The agent got an error result (bad arguments, upstream or database error, …).
    Error,
    /// The tool doesn't exist or is hidden from agents.
    UnknownTool,
    /// The tool needs approval and didn't get it (denied, timed out, cancelled).
    NotApproved,
}

/// How an approval request ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Approved,
    Denied,
    /// Nobody answered within the project's timeout.
    TimedOut,
    /// The server shut down or the client went away while waiting.
    Cancelled,
}

impl ApprovalDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            ApprovalDecision::Approved => "approved",
            ApprovalDecision::Denied => "denied",
            ApprovalDecision::TimedOut => "timed_out",
            ApprovalDecision::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "approved" => Some(ApprovalDecision::Approved),
            "denied" => Some(ApprovalDecision::Denied),
            "timed_out" => Some(ApprovalDecision::TimedOut),
            "cancelled" => Some(ApprovalDecision::Cancelled),
            _ => None,
        }
    }
}

/// The approval step of a call that needed one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRecord {
    pub decision: ApprovalDecision,
    /// Who answered: a signed-in user, `admin token`, or the MCP client.
    pub decided_by: Option<String>,
    pub wait_ms: u64,
}

impl CallStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            CallStatus::Ok => "ok",
            CallStatus::Error => "error",
            CallStatus::UnknownTool => "unknown_tool",
            CallStatus::NotApproved => "not_approved",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ok" => Some(CallStatus::Ok),
            "error" => Some(CallStatus::Error),
            "unknown_tool" => Some(CallStatus::UnknownTool),
            "not_approved" => Some(CallStatus::NotApproved),
            _ => None,
        }
    }
}

/// One tool call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallEntry {
    pub id: Uuid,
    pub project_id: Uuid,
    pub ts: DateTime<Utc>,
    /// Kept as text: the tool may be deleted later. `None` for unknown tools.
    pub tool_id: Option<Uuid>,
    pub tool_name: String,
    /// `mcp` (an agent) or `console` (the UI test console).
    pub origin: String,
    /// MCP client as `name version`, or the signed-in user for the console.
    pub client: Option<String>,
    /// Arguments with secret-looking values redacted, cut to
    /// [`MAX_LOGGED_ARGS_CHARS`] (then stored as a JSON string).
    pub arguments: Value,
    pub status: CallStatus,
    pub duration_ms: u64,
    /// Length of the text handed back, in characters.
    pub response_chars: u64,
    /// HTTP status from the upstream API, for HTTP tools that got a response.
    pub upstream_status: Option<u16>,
    /// Start of the error text for failed calls.
    pub error: Option<String>,
    /// Set when the tool needed approval.
    #[serde(default)]
    pub approval: Option<ApprovalRecord>,
    /// Name of the access key the call came in with.
    #[serde(default)]
    pub api_key: Option<String>,
}

/// Whether an argument name looks like it holds a secret.
pub fn is_secret_name(name: &str) -> bool {
    const WORDS: &[&str] = &[
        "password",
        "passwd",
        "secret",
        "token",
        "apikey",
        "api_key",
        "api-key",
        "authorization",
        "cookie",
        "credential",
        "private_key",
        "privatekey",
    ];
    let n = name.to_ascii_lowercase();
    WORDS.iter().any(|w| n.contains(w))
}

/// Arguments as recorded: secret-looking keys (at any depth) redacted, then
/// cut to [`MAX_LOGGED_ARGS_CHARS`].
pub fn logged_arguments(args: &Map<String, Value>) -> Value {
    let redacted = redact(&Value::Object(args.clone()));
    let text = redacted.to_string();
    if text.chars().count() <= MAX_LOGGED_ARGS_CHARS {
        redacted
    } else {
        let cut: String = text.chars().take(MAX_LOGGED_ARGS_CHARS).collect();
        Value::String(format!("{cut}…"))
    }
}

fn redact(v: &Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, v)| {
                    let v = if is_secret_name(k) && !v.is_null() {
                        Value::String(REDACTED.into())
                    } else {
                        redact(v)
                    };
                    (k.clone(), v)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(redact).collect()),
        other => other.clone(),
    }
}

/// `text` cut to [`MAX_LOGGED_ERROR_CHARS`].
pub fn logged_error(text: &str) -> String {
    if text.chars().count() <= MAX_LOGGED_ERROR_CHARS {
        text.to_owned()
    } else {
        let cut: String = text.chars().take(MAX_LOGGED_ERROR_CHARS).collect();
        format!("{cut}…")
    }
}

#[cfg(test)]
#[path = "../tests/unit/calls.rs"]
mod tests;
