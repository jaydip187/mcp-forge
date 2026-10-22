use super::*;
use crate::tool::ToolInput;

fn spec(v: Value) -> HttpSpec {
    let t: ToolInput = serde_json::from_value(json!({ "name": "t", "kind": v })).unwrap();
    let crate::ToolKind::Http(h) = t.kind else {
        panic!("http tool")
    };
    h
}

#[test]
fn derives_schema_and_hides_fixed() {
    let s = spec(json!({
        "type": "http", "method": "GET", "base_url": "https://x.io", "path_template": "/u/{id}",
        "params": [
            { "name": "id", "location": "path", "required": true, "schema": { "type": "integer" }, "description": "User id" },
            { "name": "q", "location": "query" },
            { "name": "X-Version", "location": "header", "fixed_value": "2" }
        ]
    }));
    let d = s.derive_input();
    assert_eq!(
        Value::Object(d.schema),
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "integer", "description": "User id" },
                "q": { "type": "string" }
            },
            "required": ["id"],
            "additionalProperties": false
        })
    );
    assert_eq!(d.bindings.len(), 2);
    assert_eq!(
        d.bindings[0].target,
        ArgTarget::Param {
            location: ParamLocation::Path,
            name: "id".into(),
            wire: "id".into(),
        }
    );
}

#[test]
fn prefixes_only_on_collision() {
    let s = spec(json!({
        "type": "http", "method": "PUT", "base_url": "https://x.io", "path_template": "/u/{id}",
        "params": [
            { "name": "id", "location": "path", "required": true },
            { "name": "id", "location": "body" },
            { "name": "name", "location": "body" }
        ]
    }));
    let args: Vec<_> = s
        .derive_input()
        .bindings
        .into_iter()
        .map(|b| b.arg)
        .collect();
    assert_eq!(args, ["path_id", "body_id", "name"]);
}

#[test]
fn whole_body_arg() {
    let s = spec(json!({
        "type": "http", "method": "POST", "base_url": "https://x.io", "path_template": "/u",
        "body_schema": { "type": "object", "properties": { "a": { "type": "string" } } }
    }));
    let d = s.derive_input();
    assert_eq!(d.bindings[0].arg, BODY_ARG);
    assert_eq!(d.bindings[0].target, ArgTarget::WholeBody);
    assert_eq!(d.schema["required"], json!(["body"]));
}

#[test]
fn validation_catches_prefix_clash() {
    // `query_id` (literal) clashes with the prefixed form of `id` in query.
    let t: ToolInput = serde_json::from_value(json!({ "name": "t", "kind": {
        "type": "http", "method": "GET", "base_url": "https://x.io", "path_template": "/u/{id}",
        "params": [
            { "name": "id", "location": "path", "required": true },
            { "name": "id", "location": "query" },
            { "name": "query_id", "location": "header" }
        ]
    }}))
    .unwrap();
    assert!(t.validate().unwrap_err().has("kind.params"));
}
