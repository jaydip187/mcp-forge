use super::*;
use serde_json::json;

fn conn(allow: &[&str]) -> DbConnection {
    serde_json::from_value(json!({
            "id": uuid::Uuid::nil(), "project_id": uuid::Uuid::nil(), "name": "db", "driver": "postgres",
            "dsn_ref": "env:DB", "access_mode": "read_only", "row_limit": 1, "timeout_ms": 1000,
            "max_affected_rows": 1, "allow_ddl": false, "table_allowlist": allow,
            "column_masks": ["users.email"],
            "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z"
        }))
        .unwrap()
}

#[test]
fn list_filters_allowlist_system_and_pattern() {
    let raw = RawResult {
        columns: vec![],
        rows: vec![
            vec![json!("public"), json!("users"), json!("BASE TABLE")],
            vec![json!("public"), json!("orders"), json!("BASE TABLE")],
            vec![json!("public"), json!("order_totals"), json!("VIEW")],
            vec![json!("public"), json!("secrets"), json!("BASE TABLE")],
        ],
        truncated: false,
    };
    let c = conn(&["users", "public.orders", "order_totals"]);
    let out = list_tables(&c, "q", raw.clone(), None);
    let names: Vec<_> = out.rows.iter().map(|r| r[1].clone()).collect();
    assert_eq!(
        names,
        [json!("users"), json!("orders"), json!("order_totals")]
    );
    assert_eq!(out.rows[2][2], json!("view"));
    let out = list_tables(&c, "q", raw, Some("ORDER"));
    assert_eq!(out.rows.len(), 2);
}

#[test]
fn describe_reports_masks_and_ambiguity() {
    let c = conn(&[]);
    let raw = RawResult {
        columns: vec![],
        rows: vec![
            vec![
                json!("public"),
                json!("id"),
                json!("integer"),
                json!(false),
                Value::Null,
                json!(true),
            ],
            vec![
                json!("public"),
                json!("email"),
                json!("text"),
                json!(1),
                json!("''::text"),
                json!(0),
            ],
        ],
        truncated: false,
    };
    let out = describe_table(&c, "users", "q".into(), &[], raw.clone()).unwrap();
    assert_eq!(
        out.rows[0],
        [
            json!("id"),
            json!("integer"),
            json!(false),
            Value::Null,
            json!(true),
            json!(false)
        ]
    );
    assert_eq!(out.rows[1][5], json!(true), "email is masked");
    assert_eq!(out.rows[1][2], json!(true), "1 means nullable");

    let mut two = raw;
    two.rows[1][0] = json!("sales");
    assert!(describe_table(&c, "users", "q".into(), &[], two).is_err());
    let empty = RawResult::default();
    assert!(describe_table(&c, "nope", "q".into(), &[], empty).is_err());
}
