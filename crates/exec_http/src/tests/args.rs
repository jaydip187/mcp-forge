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
