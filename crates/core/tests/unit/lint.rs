use serde_json::json;
use uuid::Uuid;

use super::*;

fn tool(v: Value) -> ToolDef {
    let mut base = json!({
        "id": Uuid::nil(), "project_id": Uuid::nil(), "name": "list_open_issues",
        "description": "Open issues of one repository, newest first. Use to find work.",
        "enabled": true, "tags": [], "response_shape": null, "require_approval": false,
        "source": { "type": "manual" }, "overrides": [],
        "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z",
        "kind": { "type": "http", "method": "GET", "base_url": "https://api.example.com",
                  "path_template": "/repos/{repo}/issues/{number}", "params": [], "body_schema": null,
                  "auth_ref": null, "timeout_ms": 30000 }
    });
    for (k, x) in v.as_object().unwrap() {
        base[k] = x.clone();
    }
    serde_json::from_value(base).unwrap()
}

fn rules(l: &ToolLint) -> Vec<&'static str> {
    l.findings.iter().map(|f| f.rule).collect()
}

#[test]
fn a_good_tool_scores_100() {
    let l = lint_tool(&tool(json!({})), Some(500));
    assert_eq!(l.score, 100, "{:?}", l.findings);
}

#[test]
fn descriptions_and_names() {
    let l = lint_tool(&tool(json!({ "description": "" })), None);
    assert_eq!(rules(&l), ["missing_description"]);
    assert_eq!(l.score, 80);
    let l = lint_tool(&tool(json!({ "description": "Gets issues" })), None);
    assert_eq!(rules(&l), ["short_description"]);
    for name in ["get", "tool2", "api_call_1", "run", "op"] {
        let l = lint_tool(&tool(json!({ "name": name })), None);
        assert!(
            rules(&l).contains(&"vague_name") || name == "api_call_1",
            "{name}: {:?}",
            rules(&l)
        );
    }
    assert!(rules(&lint_tool(&tool(json!({ "name": "tool2" })), None)).contains(&"vague_name"));
    assert!(
        !rules(&lint_tool(&tool(json!({ "name": "get_customer" })), None)).contains(&"vague_name")
    );
}

#[test]
fn params() {
    let kind = json!({ "type": "http", "method": "GET", "base_url": "https://api.example.com",
        "path_template": "/issues/{id}", "body_schema": null, "auth_ref": null, "timeout_ms": 30000,
        "params": [
            { "name": "id", "location": "path", "schema": { "type": "string" }, "required": true, "description": "Issue id", "fixed_value": null },
            { "name": "state", "location": "query", "schema": { "type": "string" }, "required": false, "description": null, "fixed_value": null },
            { "name": "sort_order", "location": "query", "schema": { "type": "string", "enum": ["asc", "desc"] }, "required": false, "description": "Order", "fixed_value": null },
            { "name": "api_version", "location": "header", "schema": { "type": "string" }, "required": false, "description": null, "fixed_value": "3" }
        ] });
    let l = lint_tool(&tool(json!({ "kind": kind })), None);
    assert_eq!(
        rules(&l),
        ["undocumented_param", "enum_candidate"],
        "{:?}",
        l.findings
    );
    assert_eq!(l.findings[0].field, "params.state");
    assert_eq!(l.score, 90);
}

#[test]
fn param_hints_fit_the_parameter() {
    let hint = |name: &str, schema: Value| param_hint(name, &schema);
    assert!(hint("since", json!({ "type": "string" })).contains("ISO date"));
    assert!(hint("per_page", json!({ "type": "integer" })).contains("range and default"));
    assert!(hint("customer_id", json!({ "type": "string" })).contains("which record"));
    assert!(hint("archived", json!({ "type": "boolean" })).contains("true"));
    let login = hint("login", json!({ "type": "string" }));
    assert!(!login.contains("ISO date"), "{login}");
}

#[test]
fn chains_without_a_description_get_a_mild_finding() {
    let kind = json!({ "type": "chain", "params": [], "output": null,
        "steps": [{ "tool": "get_user", "args": "{}" }, { "tool": "list_repos", "args": "{}" }] });
    let l = lint_tool(&tool(json!({ "description": "", "kind": kind })), None);
    assert_eq!(rules(&l), ["missing_description"]);
    assert_eq!(l.findings[0].severity, Severity::Info);
    assert_eq!(l.score, 95);
}

#[test]
fn long_outputs() {
    let list = json!({ "type": "http", "method": "GET", "base_url": "https://api.example.com",
        "path_template": "/issues", "params": [], "body_schema": null, "auth_ref": null, "timeout_ms": 30000 });
    let l = lint_tool(&tool(json!({ "kind": list })), None);
    assert_eq!(rules(&l), ["long_output_unshaped"]);
    assert_eq!(l.findings[0].severity, Severity::Info);
    let l = lint_tool(&tool(json!({})), Some(12_000));
    assert_eq!(l.findings[0].severity, Severity::Warning);
    let l = lint_tool(
        &tool(json!({ "response_shape": "map({id})" })),
        Some(12_000),
    );
    assert!(l.findings.is_empty());
    assert_eq!(project_score(&[100, 80, 61]), 80);
    assert_eq!(project_score(&[]), 100);
}
