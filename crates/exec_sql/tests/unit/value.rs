use super::*;

#[test]
fn caps_long_cells() {
    let long = "x".repeat(MAX_CELL_CHARS + 5);
    let Value::String(s) = cap_cell(Value::String(long)) else {
        panic!()
    };
    assert!(s.ends_with("… [5 more chars]"), "{s}");
    assert_eq!(cap_cell(json!("short")), json!("short"));
    assert_eq!(cap_cell(json!(5)), json!(5));
    let big = json!({ "a": "y".repeat(MAX_CELL_CHARS) });
    assert!(cap_cell(big).is_string());
}
