//! The description linter: a report for the whole project (offline rules,
//! plus average response sizes from the call log), and an opt-in "Improve
//! with Claude" that asks the Anthropic API for better wording. Suggestions
//! are only returned, never saved; the API is called only when the project
//! has an Anthropic key reference (a user-configured upstream).

use std::collections::HashMap;
use std::time::Duration;

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use forge_core::{lint_tool, project_score, SecretRef, ToolDef, ToolKind};
use forge_store::{CallFilter, StoreError};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::api::{ApiError, ApiPath};
use crate::AppState;

type ApiResult<T> = Result<T, ApiError>;

/// The model asked for suggestions.
pub const IMPROVE_MODEL: &str = "claude-opus-5-5";
const ANTHROPIC_VERSION: &str = "2026-10-21";

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/projects/{project_id}/lint", get(lint_project))
        .route(
            "/projects/{project_id}/tools/{tool_id}/improve",
            post(improve_tool),
        )
}

/// Average response length per tool, from recent successful calls.
async fn average_chars(s: &AppState, project_id: Uuid) -> Result<HashMap<String, u64>, StoreError> {
    let calls = s
        .store
        .list_calls(project_id, &CallFilter::default(), 500)
        .await?;
    let mut sums: HashMap<String, (u64, u64)> = HashMap::new();
    for c in calls.iter().filter(|c| c.error.is_none()) {
        let e = sums.entry(c.tool_name.clone()).or_default();
        e.0 += c.response_chars;
        e.1 += 1;
    }
    Ok(sums
        .into_iter()
        .map(|(k, (sum, n))| (k, sum / n.max(1)))
        .collect())
}

async fn lint_project(
    State(s): State<AppState>,
    ApiPath(project_id): ApiPath<Uuid>,
) -> ApiResult<Json<Value>> {
    let project = s.store.get_project(project_id).await?;
    let tools = s.store.list_tools(project_id).await?;
    let avg = average_chars(&s, project_id).await?;
    let mut rows: Vec<(u32, Value)> = tools
        .iter()
        .map(|t| {
            let l = lint_tool(t, avg.get(&t.name).copied());
            (
                l.score,
                json!({ "id": t.id, "name": t.name, "enabled": t.enabled, "kind": t.kind.type_name(),
                        "score": l.score, "findings": l.findings }),
            )
        })
        .collect();
    let scores: Vec<u32> = rows
        .iter()
        .zip(&tools)
        .filter(|(_, t)| t.enabled)
        .map(|((score, _), _)| *score)
        .collect();
    rows.sort_by_key(|(score, _)| *score);
    Ok(Json(json!({
        "score": project_score(&scores),
        "improve_available": project.settings.anthropic_key_ref.as_deref().is_some_and(|r| !r.trim().is_empty()),
        "tools": rows.into_iter().map(|(_, v)| v).collect::<Vec<_>>(),
    })))
}

/// The arguments an agent fills in: name, description, type, allowed values.
fn params_of(t: &ToolDef) -> Vec<Value> {
    let row = |name: &str, desc: Option<&str>, schema: &Value| json!({ "name": name, "description": desc.unwrap_or(""), "type": schema.get("type"), "enum": schema.get("enum") });
    match &t.kind {
        ToolKind::Http(h) => h
            .params
            .iter()
            .filter(|p| p.fixed_value.is_none())
            .map(|p| row(&p.name, p.description.as_deref(), &p.schema))
            .collect(),
        ToolKind::Sql(s) => s
            .params()
            .iter()
            .map(|p| row(&p.name, p.description.as_deref(), &p.schema))
            .collect(),
        ToolKind::Nosql(s) => s
            .params
            .iter()
            .map(|p| row(&p.name, p.description.as_deref(), &p.schema))
            .collect(),
        ToolKind::Chain(c) => c
            .params
            .iter()
            .map(|p| row(&p.name, p.description.as_deref(), &p.schema))
            .collect(),
        ToolKind::Graphql(g) => g
            .variables
            .iter()
            .map(|v| row(&v.name, Some(&v.description), &v.json_schema()))
            .collect(),
    }
}

/// What the tool does, in terms the model can describe (no secrets: auth
/// configs are referenced by id only and aren't included).
fn tool_facts(t: &ToolDef) -> Value {
    let what = match &t.kind {
        ToolKind::Http(h) => {
            json!({ "http": format!("{} {}", format!("{:?}", h.method).to_uppercase(), h.path_template) })
        }
        ToolKind::Graphql(g) => json!({ "graphql": g.document }),
        ToolKind::Sql(s) => match &s.mode {
            forge_core::SqlMode::Query { sql, .. } => json!({ "sql": sql }),
            m => json!({ "sql_tool": m.as_str() }),
        },
        ToolKind::Nosql(n) => {
            json!({ "operation": n.operation, "target": n.target, "body": n.body })
        }
        ToolKind::Chain(c) => {
            json!({ "steps": c.steps.iter().map(|s| s.tool.clone()).collect::<Vec<_>>() })
        }
    };
    json!({ "name": t.name, "kind": t.kind.type_name(), "does": what, "description": t.description, "params": params_of(t) })
}

fn suggestion_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "description": { "type": "string" },
            "params": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": { "name": { "type": "string" }, "description": { "type": "string" } },
                    "required": ["name", "description"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["description", "params"],
        "additionalProperties": false
    })
}

const SYSTEM: &str =
    "You improve how tools describe themselves to AI agents. Given a tool and the \
problems a linter found, write a clear description (what it returns, when to use it, 1-3 \
sentences, no marketing) and a short description for each parameter (what to pass and its \
format). Keep parameter names exactly as given. Describe only what the tool's definition shows; \
don't invent behavior.";

async fn improve_tool(
    State(s): State<AppState>,
    ApiPath((project_id, tool_id)): ApiPath<(Uuid, Uuid)>,
) -> ApiResult<Json<Value>> {
    let project = s.store.get_project(project_id).await?;
    let tool = s.store.get_tool(project_id, tool_id).await?;
    let Some(key_ref) = project
        .settings
        .anthropic_key_ref
        .filter(|r| !r.trim().is_empty())
    else {
        return Err(ApiError::BadRequest(
            "Add an Anthropic API key reference in the project settings first.".into(),
        ));
    };
    let r = SecretRef::parse(&key_ref)
        .map_err(|e| ApiError::BadRequest(format!("bad key reference: {e}")))?;
    let secrets = s.secrets.clone();
    let key = tokio::task::spawn_blocking(move || secrets.resolve(&r))
        .await
        .map_err(|e| ApiError::BadRequest(format!("secret lookup failed: {e}")))?
        .map_err(|e| ApiError::BadRequest(format!("the Anthropic key isn't available: {e}")))?;
    let avg = average_chars(&s, project_id).await?;
    let findings = lint_tool(&tool, avg.get(&tool.name).copied()).findings;
    let user = json!({ "tool": tool_facts(&tool), "problems": findings }).to_string();
    let body = json!({
        "model": IMPROVE_MODEL,
        "max_tokens": 4000,
        "output_config": { "effort": "low", "format": { "type": "json_schema", "schema": suggestion_schema() } },
        // Retries a refused request on another model, routed by refusal category.
        "fallbacks": "default",
        "system": SYSTEM,
        "messages": [{ "role": "user", "content": user }],
    });
    let redact = |m: String| m.replace(&key, "***");
    let resp = reqwest::Client::builder()
        .timeout(Duration::from_secs(90))
        .build()
        .map_err(|e| ApiError::BadRequest(format!("HTTP client: {e}")))?
        .post(format!(
            "{}/v1/messages",
            s.anthropic_url.trim_end_matches('/')
        ))
        .header("x-api-key", &key)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .header("anthropic-beta", "server-side-fallback-2026-07-01")
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            ApiError::BadGateway(redact(format!(
                "couldn't reach the Anthropic API: {}",
                e.without_url()
            )))
        })?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let msg = v
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("request failed");
        return Err(ApiError::BadGateway(redact(format!(
            "Anthropic API error ({status}): {msg}"
        ))));
    }
    if v.get("stop_reason").and_then(Value::as_str) == Some("refusal") {
        return Err(ApiError::BadGateway(
            "Claude declined to suggest a description for this tool.".into(),
        ));
    }
    let text = v
        .get("content")
        .and_then(Value::as_array)
        .and_then(|blocks| {
            blocks
                .iter()
                .find(|b| b.get("type").and_then(Value::as_str) == Some("text"))
        })
        .and_then(|b| b.get("text").and_then(Value::as_str))
        .unwrap_or("");
    let suggested: Value = serde_json::from_str(text).map_err(|_| {
        ApiError::BadGateway("Claude's answer wasn't the expected JSON; try again.".into())
    })?;
    let current = json!({
        "description": tool.description,
        "params": params_of(&tool).iter().map(|p| json!({ "name": p["name"], "description": p["description"] })).collect::<Vec<_>>(),
    });
    Ok(Json(json!({
        "current": current,
        "suggested": suggested,
        "model": v.get("model").cloned().unwrap_or(json!(IMPROVE_MODEL)),
    })))
}
