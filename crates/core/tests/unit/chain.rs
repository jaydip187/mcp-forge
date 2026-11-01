use super::*;

fn spec(v: Value) -> ChainSpec {
    serde_json::from_value(v).unwrap()
}

#[test]
fn validation() {
    let ok = spec(json!({
        "params": [{ "name": "login", "required": true }],
        "steps": [{ "tool": "get_user", "args": "{login: .input.login}" }, { "tool": "list_repos", "args": "{owner: .last.login}" }]
    }));
    let mut e = ValidationErrors::default();
    ok.validate_into(&mut e);
    assert!(e.is_empty(), "{e:?}");
    assert_eq!(ok.derive_input().schema["required"], json!(["login"]));

    let bad = spec(json!({
        "params": [{ "name": "dry_run" }, { "name": "x", "schema": { "type": "array" } }],
        "steps": [{ "tool": "bad name" }]
    }));
    let mut e = ValidationErrors::default();
    bad.validate_into(&mut e);
    for f in [
        "kind.params[0].name",
        "kind.params[1].schema",
        "kind.steps[0].tool",
    ] {
        assert!(e.has(f), "{f}: {e:?}");
    }
    let mut e = ValidationErrors::default();
    spec(json!({ "steps": [] })).validate_into(&mut e);
    assert!(e.has("kind.steps"));
    let many: Vec<Value> = (0..11).map(|_| json!({ "tool": "t" })).collect();
    let mut e = ValidationErrors::default();
    spec(json!({ "steps": many })).validate_into(&mut e);
    assert!(e.has("kind.steps"));
}

#[test]
fn context() {
    let input = json!({ "a": 1 }).as_object().unwrap().clone();
    assert_eq!(
        step_context(&input, &[json!(1), json!({ "x": 2 })]),
        json!({ "input": { "a": 1 }, "steps": [1, { "x": 2 }], "last": { "x": 2 } })
    );
    assert_eq!(step_context(&input, &[])["last"], Value::Null);
}
