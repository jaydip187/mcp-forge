use super::*;

fn spec(op: &str, target: &str, body: Value, params: Value) -> NosqlSpec {
    serde_json::from_value(json!({
        "connection_id": Uuid::nil(), "operation": op, "target": target,
        "body": body, "params": params
    }))
    .unwrap()
}

fn args(v: Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}

#[test]
fn templates_render_typed_values_and_never_keys() {
    let body = json!({
        "filter": { "age": { "$gte": "{{min}}" }, "name": "{{name}}", "{{name}}": 1 },
        "note": "hello {{name}}, you are {{min}}",
        "list": ["{{ min }}", "x"]
    });
    let out = render_value(&body, &args(json!({ "min": 21, "name": { "$ne": null } })));
    assert_eq!(out["filter"]["age"]["$gte"], json!(21), "typed, not text");
    assert_eq!(out["filter"]["{{name}}"], json!(1), "keys are left alone");
    assert_eq!(out["list"][0], json!(21));
    // Even a (never validated) object argument lands as a value, not as operators
    // merged into the filter.
    assert_eq!(out["filter"]["name"], json!({ "$ne": null }));
    let out = render_value(&body, &args(json!({ "min": 3, "name": "Ada" })));
    assert_eq!(out["note"], json!("hello Ada, you are 3"));
    assert_eq!(
        render_value(&json!("{{gone}}"), &args(json!({}))),
        Value::Null
    );
    assert_eq!(
        render_target("user:{{id}}:profile", &args(json!({ "id": 42 }))),
        "user:42:profile"
    );
}

#[test]
fn validation_matches_params_and_placeholders() {
    let ok = spec(
        "find",
        "users",
        json!({ "filter": { "email": "{{email}}" } }),
        json!([{ "name": "email" }]),
    );
    let mut e = ValidationErrors::default();
    ok.validate_into(&mut e);
    assert!(e.is_empty(), "{e:?}");

    let bad = spec(
        "find",
        "",
        json!({ "filter": { "email": "{{email}}" } }),
        json!([{ "name": "unused" }, { "name": "n", "schema": { "type": "object" } }]),
    );
    let mut e = ValidationErrors::default();
    bad.validate_into(&mut e);
    for f in [
        "kind.target",
        "kind.params[0].name",
        "kind.params[1].schema",
        "kind.params",
    ] {
        assert!(e.has(f), "{f}: {e:?}");
    }
    let mut e = ValidationErrors::default();
    spec("find", "users", json!("nope"), json!([])).validate_into(&mut e);
    assert!(e.has("kind.body"));
    // Exploration tools carry no target or body.
    let mut e = ValidationErrors::default();
    spec(RUN_QUERY, "", Value::Null, json!([])).validate_into(&mut e);
    assert!(e.is_empty(), "{e:?}");
}

#[test]
fn operations_per_driver() {
    assert_eq!(op_class(Driver::Mongodb, "find"), Some(OpClass::Read));
    assert_eq!(
        op_class(Driver::Mongodb, "delete_many"),
        Some(OpClass::Write)
    );
    assert_eq!(
        op_class(Driver::Mongodb, "FIND"),
        None,
        "Mongo names are exact"
    );
    assert_eq!(op_class(Driver::Redis, "hgetall"), Some(OpClass::Read));
    assert_eq!(op_class(Driver::Redis, "FLUSHALL"), None);
    assert_eq!(op_class(Driver::Redis, "EVAL"), None);
    assert_eq!(op_class(Driver::Dynamodb, "Query"), Some(OpClass::Read));
    assert_eq!(
        op_class(Driver::Elasticsearch, "delete"),
        Some(OpClass::Write)
    );
    assert!(nosql_operations(Driver::Postgres).is_empty());
    assert!(!Driver::Redis.is_sql() && Driver::Mssql.is_sql());
    let s = spec("update_many", "users", json!({}), json!([]));
    assert!(s.writes(Driver::Mongodb));
}

#[test]
fn inputs() {
    let d = spec(RUN_QUERY, "", Value::Null, json!([])).derive_input();
    assert_eq!(d.schema["required"], json!(["operation", "target"]));
    assert_eq!(d.bindings.len(), 3);
    let d = spec(
        "GET",
        "user:{{id}}",
        Value::Null,
        json!([{ "name": "id", "schema": { "type": "integer" }, "required": true, "description": "User id" }]),
    )
    .derive_input();
    assert_eq!(
        d.schema["properties"]["id"],
        json!({ "type": "integer", "description": "User id" })
    );
    assert_eq!(d.schema["required"], json!(["id"]));
}
