use super::*;
use serde_json::json;

const OPTS: ShapeOptions = ShapeOptions {
    strip_nulls: false,
    max_chars: 1_000,
};

#[test]
fn jq_expressions() {
    let users = json!([
        { "id": 1, "name": "Ada", "email": "ada@x.io", "meta": { "plan": "pro" } },
        { "id": 2, "name": "Linus", "email": null, "meta": { "plan": "free" } }
    ]);
    assert_eq!(
        apply("map({id, name})", &users).unwrap(),
        json!([{ "id": 1, "name": "Ada" }, { "id": 2, "name": "Linus" }])
    );
    assert_eq!(apply(".[0].meta.plan", &users).unwrap(), json!("pro"));
    assert_eq!(
        apply(".[] | .name", &users).unwrap(),
        json!(["Ada", "Linus"]),
        "many outputs → array"
    );
    assert_eq!(
        apply("empty", &users).unwrap(),
        Value::Null,
        "no output → null"
    );
    assert_eq!(apply("length", &users).unwrap(), json!(2));
    assert_eq!(
        apply("[.[] | select(.meta.plan == \"pro\") | .email]", &users).unwrap(),
        json!(["ada@x.io"])
    );
    assert_eq!(apply("map(.id) | add", &users).unwrap(), json!(3));
}

#[test]
fn bad_expressions_are_reported() {
    assert!(check("map({id, name})").is_ok());
    let e = check(".foo | ").unwrap_err();
    assert!(e.contains("can't parse"), "{e}");
    let e = check("no_such_function(1)").unwrap_err();
    assert!(e.contains("can't compile"), "{e}");
    let e = apply("error(\"boom\")", &json!({ "a": 1 })).unwrap_err();
    assert!(e.contains("boom"), "{e}");
    let e = apply("repeat(1)", &json!(null)).unwrap_err();
    assert!(e.contains("more than"), "{e}");
}

#[test]
fn strip_nulls_recursively() {
    let mut v = json!({ "a": null, "b": { "c": null, "d": 1 }, "e": [null, { "f": null }] });
    strip_nulls(&mut v);
    assert_eq!(v, json!({ "b": { "d": 1 }, "e": [null, {}] }));
}

#[test]
fn truncation_notice() {
    let (t, cut) = truncate("héllo wörld", 5);
    assert!(cut);
    assert!(
        t.starts_with("héllo\n\n[Response truncated: showing the first 5 of 11 characters."),
        "{t}"
    );
    let (t, cut) = truncate("short", 5);
    assert_eq!((t.as_str(), cut), ("short", false));
}

#[test]
fn shape_body_pipeline() {
    let body = r#"{"items":[{"id":1,"note":null},{"id":2,"note":"x"}],"next":null}"#;
    let s = shape_body(
        body,
        Some("application/json"),
        Some(".items | map(.id)"),
        &OPTS,
    );
    assert_eq!(s.text, "[1,2]");
    assert!(s.transformed && !s.truncated && s.shape_error.is_none());

    let strip = ShapeOptions {
        strip_nulls: true,
        max_chars: 1_000,
    };
    let s = shape_body(body, Some("application/json"), None, &strip);
    assert_eq!(s.text, r#"{"items":[{"id":1},{"id":2,"note":"x"}]}"#);

    // No shaping configured: the body passes through untouched.
    let s = shape_body(body, Some("application/json"), None, &OPTS);
    assert_eq!((s.text.as_str(), s.transformed), (body, false));

    // A failing expression falls back to the raw body with a note.
    let s = shape_body(
        body,
        Some("application/json"),
        Some("error(\"no such field\")"),
        &OPTS,
    );
    assert!(s.shape_error.is_some());
    assert!(
        s.text.starts_with("[response_shape not applied:"),
        "{}",
        s.text
    );
    assert!(s.text.ends_with(body));

    // Non-JSON with an expression: note, then the text.
    let s = shape_body("plain text", Some("text/plain"), Some("."), &OPTS);
    assert_eq!(s.shape_error.as_deref(), Some("the response isn't JSON"));

    // Size cap applies after shaping.
    let tiny = ShapeOptions {
        strip_nulls: false,
        max_chars: 10,
    };
    let s = shape_body(body, Some("application/json"), None, &tiny);
    assert!(s.truncated && s.text.starts_with(r#"{"items":["#));
}
