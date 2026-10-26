use super::*;
use crate::schema::ArgTarget;
use serde_json::json;

fn binding(arg: &str, required: bool) -> ArgBinding {
    ArgBinding {
        arg: arg.into(),
        target: ArgTarget::WholeBody,
        required,
    }
}

fn obj(v: Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}

#[test]
fn checks_unknown_missing_type_enum() {
    let bindings = [binding("id", true), binding("mode", false)];
    let schema = obj(json!({ "properties": {
        "id": { "type": "integer" },
        "mode": { "type": "string", "enum": ["a", "b"] }
    }}));
    check_args(&bindings, &schema, &obj(json!({ "id": 3, "mode": "a" }))).unwrap();
    check_args(&bindings, &schema, &obj(json!({ "id": 3.0 }))).unwrap();

    let e = check_args(&bindings, &schema, &obj(json!({ "mode": "z", "extra": 1 }))).unwrap_err();
    assert!(e.has("id") && e.has("mode") && e.has("extra"), "{e}");
    let e = check_args(&bindings, &schema, &obj(json!({ "id": "3" }))).unwrap_err();
    assert!(
        e.to_string().contains("expected integer, got string"),
        "{e}"
    );
}
