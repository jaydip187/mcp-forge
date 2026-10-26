use super::*;
use serde_json::json;

fn tool(spec: Value) -> ToolInput {
    serde_json::from_value(json!({ "name": "get_user", "kind": spec })).unwrap()
}

fn valid_spec() -> Value {
    json!({
        "type": "http",
        "method": "GET",
        "base_url": "https://api.example.com/v1",
        "path_template": "/users/{id}",
        "params": [
            { "name": "id", "location": "path", "required": true },
            { "name": "verbose", "location": "query", "schema": { "type": "boolean" } }
        ]
    })
}

fn errs(spec: Value) -> ValidationErrors {
    tool(spec).validate().unwrap_err()
}

#[test]
fn valid_tool_with_defaults() {
    let t = tool(valid_spec());
    t.validate().unwrap();
    assert!(t.enabled);
    let ToolKind::Http(h) = &t.kind else {
        panic!("http tool")
    };
    assert_eq!(h.timeout_ms, DEFAULT_TIMEOUT_MS);
    assert_eq!(h.params[0].schema, json!({ "type": "string" }));
}

#[test]
fn serde_round_trip() {
    let t = tool(valid_spec());
    let back: ToolInput = serde_json::from_value(serde_json::to_value(&t).unwrap()).unwrap();
    assert_eq!(t, back);
    assert_eq!(
        serde_json::to_value(Source::Manual).unwrap(),
        json!({ "type": "manual" })
    );
}

#[test]
fn rejects_bad_tool_name() {
    let mut t = tool(valid_spec());
    t.name = "get user!".into();
    assert!(t.validate().unwrap_err().has("name"));
}

#[test]
fn placeholder_parsing() {
    assert_eq!(path_placeholders("/a/{x}/b/{y}").unwrap(), ["x", "y"]);
    assert!(path_placeholders("/a/{x").is_err());
    assert!(path_placeholders("/a/x}").is_err());
    assert!(path_placeholders("/a/{}").is_err());
    assert!(path_placeholders("/a/{x{y}}").is_err());
}

#[test]
fn path_params_must_match_template() {
    let mut s = valid_spec();
    s["path_template"] = json!("/users/{user_id}");
    let e = errs(s);
    assert!(e.has("kind.params"), "{e}"); // {user_id} undeclared
    assert!(e.has("kind.params[0].name"), "{e}"); // id not in template
}

#[test]
fn path_params_must_be_required() {
    let mut s = valid_spec();
    s["params"][0]["required"] = json!(false);
    assert!(errs(s.clone()).has("kind.params[0].required"));
    s["params"][0]["fixed_value"] = json!("42");
    tool(s).validate().unwrap();
}

#[test]
fn duplicate_params_per_location() {
    let mut s = valid_spec();
    s["params"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "name": "verbose", "location": "query" }));
    assert!(errs(s.clone()).has("kind.params[2].name"));
    // Same name in a different location is allowed.
    s["params"][2]["location"] = json!("header");
    tool(s).validate().unwrap();
}

#[test]
fn wire_names() {
    let mut s = valid_spec();
    s["params"][1]["name"] = json!("filter_name");
    s["params"][1]["wire_name"] = json!("filter[name]");
    tool(s.clone()).validate().unwrap();
    // Same wire name twice in one location is a duplicate.
    s["params"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "name": "other", "location": "query", "wire_name": "filter[name]" }));
    assert!(errs(s.clone()).has("kind.params[2].name"));
    // Not on path params.
    let mut s = valid_spec();
    s["params"][0]["wire_name"] = json!("user-id");
    assert!(errs(s).has("kind.params[0].wire_name"));
}

#[test]
fn base_url_rules() {
    for bad in [
        "not a url",
        "ftp://x.com",
        "https://user:pw@x.com",
        "https://x.com?a=1",
        "https://x.com#f",
    ] {
        let mut s = valid_spec();
        s["base_url"] = json!(bad);
        assert!(errs(s).has("kind.base_url"), "{bad}");
    }
}

#[test]
fn path_template_rules() {
    for bad in ["users/{id}", "/users/{id}?x=1", "/users/{id}/{id}"] {
        let mut s = valid_spec();
        s["path_template"] = json!(bad);
        assert!(errs(s).has("kind.path_template"), "{bad}");
    }
}

#[test]
fn body_schema_and_body_params_exclusive() {
    let mut s = valid_spec();
    s["method"] = json!("POST");
    s["body_schema"] = json!({ "type": "object" });
    tool(s.clone()).validate().unwrap();
    s["params"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "name": "title", "location": "body" }));
    assert!(errs(s).has("kind.body_schema"));
}

#[test]
fn schema_must_be_object_and_timeout_in_range() {
    let mut s = valid_spec();
    s["params"][1]["schema"] = json!("string");
    s["timeout_ms"] = json!(0);
    let e = errs(s);
    assert!(e.has("kind.params[1].schema") && e.has("kind.timeout_ms"));
}
