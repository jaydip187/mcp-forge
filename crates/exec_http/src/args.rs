//! Agent argument checking and value → wire-string conversion.

use std::collections::HashSet;

use forge_core::{ArgBinding, ValidationErrors};
use serde_json::{Map, Value};

/// Checks arguments against the derived bindings and schema: unknown names,
/// missing required args, top-level JSON type and `enum`. Each error names the
/// offending argument.
pub fn check_args(
    bindings: &[ArgBinding],
    schema: &Map<String, Value>,
    args: &Map<String, Value>,
) -> Result<(), ValidationErrors> {
    let mut errs = ValidationErrors::default();
    let known: HashSet<&str> = bindings.iter().map(|b| b.arg.as_str()).collect();
    let mut expected: Vec<&str> = known.iter().copied().collect();
    expected.sort_unstable();

    let mut unknown: Vec<&String> = args
        .keys()
        .filter(|k| !known.contains(k.as_str()))
        .collect();
    unknown.sort();
    for k in unknown {
        let hint = if expected.is_empty() {
            "this tool takes no arguments".to_owned()
        } else {
            format!("expected one of: {}", expected.join(", "))
        };
        errs.push(k.clone(), format!("unknown argument; {hint}"));
    }

    let props = schema.get("properties").and_then(Value::as_object);
    for b in bindings {
        match args.get(&b.arg) {
            None | Some(Value::Null) if b.required => {
                errs.push(b.arg.clone(), "required argument is missing");
            }
            None | Some(Value::Null) => {}
            Some(v) => {
                if let Some(s) = props.and_then(|p| p.get(&b.arg)) {
                    if let Some(msg) = schema_mismatch(s, v) {
                        errs.push(b.arg.clone(), msg);
                    }
                }
            }
        }
    }
    errs.into_result()
}

fn schema_mismatch(schema: &Value, v: &Value) -> Option<String> {
    let types: Vec<&str> = match schema.get("type") {
        Some(Value::String(t)) => vec![t.as_str()],
        Some(Value::Array(ts)) => ts.iter().filter_map(Value::as_str).collect(),
        _ => vec![],
    };
    if !types.is_empty() && !types.iter().any(|t| type_matches(t, v)) {
        return Some(format!(
            "expected {}, got {}",
            types.join(" or "),
            json_type_name(v)
        ));
    }
    if let Some(Value::Array(allowed)) = schema.get("enum") {
        if !allowed.contains(v) {
            let list: Vec<String> = allowed.iter().map(Value::to_string).collect();
            return Some(format!("must be one of: {}", list.join(", ")));
        }
    }
    None
}

fn type_matches(t: &str, v: &Value) -> bool {
    match t {
        "string" => v.is_string(),
        "integer" => v.is_i64() || v.is_u64() || v.as_f64().is_some_and(|f| f.fract() == 0.0),
        "number" => v.is_number(),
        "boolean" => v.is_boolean(),
        "array" => v.is_array(),
        "object" => v.is_object(),
        "null" => v.is_null(),
        _ => true,
    }
}

fn json_type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Scalar → text as it should appear on the wire. Objects become JSON text.
pub fn scalar_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Values for a query param: arrays repeat the key, everything else is one value.
pub fn query_values(v: &Value) -> Vec<String> {
    match v {
        Value::Array(items) => items.iter().map(scalar_to_string).collect(),
        other => vec![scalar_to_string(other)],
    }
}

/// Value for a path segment or header: arrays are comma-joined.
pub fn joined_value(v: &Value) -> String {
    match v {
        Value::Array(items) => items
            .iter()
            .map(scalar_to_string)
            .collect::<Vec<_>>()
            .join(","),
        other => scalar_to_string(other),
    }
}

/// Percent-encodes everything except RFC 3986 unreserved characters, so a
/// value can never introduce `/`, `?`, `#` or `..` structure into the path.
pub fn encode_path_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    // A segment of only dots would still be interpreted as navigation.
    if !out.is_empty() && out.bytes().all(|b| b == b'.') {
        return out.replace('.', "%2E");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_core::ArgTarget;
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

        let e =
            check_args(&bindings, &schema, &obj(json!({ "mode": "z", "extra": 1 }))).unwrap_err();
        assert!(e.has("id") && e.has("mode") && e.has("extra"), "{e}");
        let e = check_args(&bindings, &schema, &obj(json!({ "id": "3" }))).unwrap_err();
        assert!(
            e.to_string().contains("expected integer, got string"),
            "{e}"
        );
    }

    #[test]
    fn path_encoding_blocks_traversal() {
        assert_eq!(encode_path_segment("a b/c?d#e"), "a%20b%2Fc%3Fd%23e");
        assert_eq!(encode_path_segment(".."), "%2E%2E");
        assert_eq!(encode_path_segment("v1.2"), "v1.2");
        assert_eq!(encode_path_segment("ü"), "%C3%BC");
    }

    #[test]
    fn value_conversions() {
        assert_eq!(query_values(&json!([1, "a"])), ["1", "a"]);
        assert_eq!(joined_value(&json!([1, 2])), "1,2");
        assert_eq!(scalar_to_string(&json!(true)), "true");
        assert_eq!(scalar_to_string(&json!({ "a": 1 })), r#"{"a":1}"#);
    }
}
