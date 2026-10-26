use super::*;
use serde_json::json;

fn tool(desc: &str, path: &str) -> ToolInput {
    serde_json::from_value(json!({
        "name": "get_user",
        "description": desc,
        "kind": {
            "type": "http", "method": "GET", "base_url": "https://x.io",
            "path_template": path,
            "params": [{ "name": "id", "location": "path", "required": true }]
        }
    }))
    .unwrap()
}

#[test]
fn detects_changes() {
    let a = tool("one", "/u/{id}");
    let mut b = a.clone();
    assert!(changed_fields(&a, &b).is_empty());
    b.description = "two".into();
    b.enabled = false;
    let ToolKind::Http(h) = &mut b.kind else {
        panic!("http tool")
    };
    h.timeout_ms = 5;
    assert_eq!(
        changed_fields(&a, &b),
        ["description", "enabled", "kind.timeout_ms"]
    );
}

#[test]
fn merge_keeps_overridden_fields_only() {
    let mut current = tool("my words", "/u/{id}");
    current.enabled = false;
    let base = tool("spec words v2", "/users/{id}");
    let mut ov = Overrides::default();
    ov.fields.insert("description".into());

    let merged = merge_overrides(&current, &ov, &base);
    assert_eq!(merged.description, "my words");
    assert!(merged.enabled, "enabled wasn't overridden, spec value wins");
    let ToolKind::Http(h) = &merged.kind else {
        panic!("http tool")
    };
    assert_eq!(h.path_template, "/users/{id}");
}

#[test]
fn params_and_path_travel_together() {
    let current = tool("d", "/u/{id}");
    let base = tool("d", "/users/{user}");
    let mut ov = Overrides::default();
    ov.fields.insert("kind.params".into());
    let merged = merge_overrides(&current, &ov, &base);
    let ToolKind::Http(h) = &merged.kind else {
        panic!("http tool")
    };
    assert_eq!(h.path_template, "/u/{id}");
    merged.validate().unwrap();
}
