//! The description linter: offline checks on how well a tool explains itself
//! to an agent, with a score and a hint per finding. Nothing here calls out.

use serde::Serialize;
use serde_json::Value;

use crate::db::SqlMode;
use crate::tool::{ToolDef, ToolKind};
use crate::HttpMethod;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LintFinding {
    pub rule: &'static str,
    pub severity: Severity,
    /// The field to fix (`description`, `name`, `params.status`, `response_shape`).
    pub field: String,
    pub message: String,
    pub hint: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolLint {
    pub score: u32,
    pub findings: Vec<LintFinding>,
}

/// Responses longer than this on average deserve a response shape.
pub const LONG_RESPONSE_CHARS: u64 = 4_000;
const SHORT_DESCRIPTION_CHARS: usize = 40;

/// Names that say nothing about what a tool does.
const VAGUE_NAMES: &[&str] = &[
    "get",
    "post",
    "put",
    "patch",
    "delete",
    "query",
    "run",
    "execute",
    "call",
    "do",
    "fetch",
    "request",
    "api",
    "tool",
    "op",
    "operation",
    "endpoint",
    "func",
    "function",
    "action",
    "test",
    "untitled",
    "new_tool",
    "data",
    "handler",
    "default",
    "misc",
];

/// Parameter names that usually hold one of a few values.
const ENUM_HINTS: &[&str] = &[
    "status",
    "state",
    "type",
    "kind",
    "sort",
    "order",
    "direction",
    "format",
    "mode",
    "level",
    "priority",
    "category",
    "visibility",
    "role",
];

fn vague(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    let stem = n.trim_end_matches(|c: char| c.is_ascii_digit() || c == '_' || c == '-');
    n.len() < 4 || VAGUE_NAMES.contains(&stem) || stem.is_empty()
}

/// `(name, description, schema)` of the arguments an agent fills in.
fn agent_params(t: &ToolDef) -> Vec<(String, Option<String>, Value)> {
    match &t.kind {
        ToolKind::Http(h) => h
            .params
            .iter()
            .filter(|p| p.fixed_value.is_none())
            .map(|p| (p.name.clone(), p.description.clone(), p.schema.clone()))
            .collect(),
        ToolKind::Sql(s) => s
            .params()
            .iter()
            .map(|p| (p.name.clone(), p.description.clone(), p.schema.clone()))
            .collect(),
        ToolKind::Nosql(s) => s
            .params
            .iter()
            .map(|p| (p.name.clone(), p.description.clone(), p.schema.clone()))
            .collect(),
        ToolKind::Chain(c) => c
            .params
            .iter()
            .map(|p| (p.name.clone(), p.description.clone(), p.schema.clone()))
            .collect(),
        ToolKind::Graphql(g) => g
            .variables
            .iter()
            .map(|v| {
                let d = (!v.description.is_empty()).then(|| v.description.clone());
                (v.name.clone(), d, v.json_schema())
            })
            .collect(),
    }
}

/// Tools whose descriptions are generated from their settings.
fn generated_description(t: &ToolDef) -> bool {
    match &t.kind {
        ToolKind::Sql(s) => !matches!(s.mode, SqlMode::Query { .. }),
        ToolKind::Nosql(s) => s.is_exploration(),
        _ => false,
    }
}

/// A fix hint for an undocumented parameter, from its name and type.
fn param_hint(name: &str, schema: &Value) -> &'static str {
    let n = name.to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| n.contains(w));
    if schema.get("enum").is_some() {
        return "Say what each allowed value means.";
    }
    match schema.get("type").and_then(Value::as_str) {
        Some("boolean") => "Say what true does, e.g. \"true to include archived items\".",
        Some("integer" | "number") if has(&["limit", "per_page", "page_size", "count", "max"]) => {
            "Say the range and default, e.g. \"1–100, default 30\"."
        }
        Some("integer" | "number") if has(&["page", "offset"]) => {
            "Say where counting starts, e.g. \"Page number, starting at 1\"."
        }
        Some("integer" | "number") if n == "id" || n.ends_with("_id") => {
            "Say which record it identifies and where agents get it, e.g. \"Order id from list_orders\"."
        }
        Some("integer" | "number") => "Say the unit and range, e.g. \"Minutes, 1–60\".",
        Some("array") => "Say what each item is, e.g. \"Label names, like bug\".",
        Some("object") => "List the fields it expects.",
        _ if has(&["date", "time", "since", "until", "before", "after"]) || n.ends_with("_at") => {
            "Give the format, e.g. \"ISO date, like 2026-01-31\"."
        }
        _ if has(&["email"]) => "Say what it matches, e.g. \"Customer email, exact match\".",
        _ if n == "id" || n.ends_with("_id") || name.ends_with("Id") => {
            "Say which record it identifies and where agents get it, e.g. \"Order id from list_orders\"."
        }
        _ if matches!(n.as_str(), "q" | "query" | "search" | "term" | "keyword" | "keywords") => {
            "Say what it searches, e.g. \"Words to find in titles and bodies\"."
        }
        _ if has(&["url", "uri", "link"]) => "Say which URL, e.g. \"Full https URL of the page\".",
        _ => "Say what to pass, with an example value, e.g. \"Account login, like ada\".",
    }
}

/// Lists everything (no WHERE / filter / page parameter in sight).
fn lists_things(t: &ToolDef) -> bool {
    match &t.kind {
        ToolKind::Http(h) => {
            h.method == HttpMethod::Get && !h.path_template.trim_end_matches('/').ends_with('}')
        }
        ToolKind::Sql(s) => matches!(s.mode, SqlMode::RunSql),
        ToolKind::Nosql(s) => matches!(
            s.operation.as_str(),
            "find" | "aggregate" | "search" | "Scan" | "Query" | "run_query"
        ),
        _ => false,
    }
}

/// Lints one tool. `avg_response_chars` comes from the call log, when there
/// are calls.
pub fn lint_tool(t: &ToolDef, avg_response_chars: Option<u64>) -> ToolLint {
    let mut f = Vec::new();
    let desc = t.description.trim();
    if !generated_description(t) {
        if desc.is_empty() && matches!(t.kind, ToolKind::Chain(_)) {
            f.push(LintFinding {
                rule: "missing_description",
                severity: Severity::Info,
                field: "description".into(),
                message: "Agents see a generated line listing the steps (\"Runs 2 steps in order: …\")."
                    .into(),
                hint: "A sentence on what the chain is for helps agents pick it, e.g. \"A user's profile and their repositories in one call.\"".into(),
            });
        } else if desc.is_empty() {
            f.push(LintFinding {
                rule: "missing_description",
                severity: Severity::Warning,
                field: "description".into(),
                message: "No description: agents only see the name (or a generated line).".into(),
                hint: "Say what it returns and when to use it, e.g. \"Open issues of one repository, newest first. Use to find work on a project.\"".into(),
            });
        } else if desc.chars().count() < SHORT_DESCRIPTION_CHARS
            || desc.split_whitespace().count() < 6
        {
            f.push(LintFinding {
                rule: "short_description",
                severity: Severity::Warning,
                field: "description".into(),
                message: format!(
                    "The description is very short ({} characters).",
                    desc.chars().count()
                ),
                hint: "Add what comes back and when an agent should pick this tool over others."
                    .into(),
            });
        }
    }
    if vague(&t.name) {
        f.push(LintFinding {
            rule: "vague_name",
            severity: Severity::Warning,
            field: "name".into(),
            message: format!("`{}` doesn't say what the tool does.", t.name),
            hint: "Use a verb and a thing, e.g. `list_open_issues` or `get_customer_by_email`."
                .into(),
        });
    }
    for (name, description, schema) in agent_params(t) {
        if description.as_deref().is_none_or(|d| d.trim().is_empty()) {
            f.push(LintFinding {
                rule: "undocumented_param",
                severity: Severity::Info,
                field: format!("params.{name}"),
                message: format!("`{name}` has no description."),
                hint: param_hint(&name, &schema).into(),
            });
        }
        let lower = name.to_ascii_lowercase();
        let is_string = schema
            .get("type")
            .and_then(Value::as_str)
            .is_none_or(|t| t == "string");
        if is_string
            && schema.get("enum").is_none()
            && ENUM_HINTS
                .iter()
                .any(|h| lower == *h || lower.ends_with(&format!("_{h}")))
        {
            f.push(LintFinding {
                rule: "enum_candidate",
                severity: Severity::Info,
                field: format!("params.{name}"),
                message: format!("`{name}` probably takes one of a few values."),
                hint: "List the allowed values so agents don't guess, e.g. open, closed, all."
                    .into(),
            });
        }
    }
    let shaped = t
        .response_shape
        .as_deref()
        .is_some_and(|s| !s.trim().is_empty());
    if !shaped {
        match avg_response_chars {
            Some(avg) if avg > LONG_RESPONSE_CHARS => f.push(LintFinding {
                rule: "long_output_unshaped",
                severity: Severity::Warning,
                field: "response_shape".into(),
                message: format!("Responses average {avg} characters and nothing trims them."),
                hint: "Add a response shape (jq) that keeps only what agents need, e.g. `map({id, title, state})`.".into(),
            }),
            None if lists_things(t) => f.push(LintFinding {
                rule: "long_output_unshaped",
                severity: Severity::Info,
                field: "response_shape".into(),
                message: "This lists things and may return a lot.".into(),
                hint: "Consider a response shape (jq) that keeps only the useful fields.".into(),
            }),
            _ => {}
        }
    }
    let penalty: u32 = f
        .iter()
        .map(|x| match x.severity {
            Severity::Warning => 20,
            Severity::Info => 5,
        })
        .sum();
    ToolLint {
        score: 100u32.saturating_sub(penalty),
        findings: f,
    }
}

/// Average of the tool scores (100 with no tools).
pub fn project_score(scores: &[u32]) -> u32 {
    if scores.is_empty() {
        return 100;
    }
    let sum: u32 = scores.iter().sum();
    (sum as f64 / scores.len() as f64).round() as u32
}

#[cfg(test)]
#[path = "../tests/unit/lint.rs"]
mod tests;
