//! Value → wire-string conversion.

use serde_json::Value;

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
    use serde_json::json;

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
