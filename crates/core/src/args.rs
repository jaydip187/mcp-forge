//! Agent argument checking shared by every executor.

use std::collections::HashSet;

use serde_json::{Map, Value};

use crate::schema::ArgBinding;
use crate::validate::ValidationErrors;

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

#[cfg(test)]
#[path = "tests/args.rs"]
mod tests;
