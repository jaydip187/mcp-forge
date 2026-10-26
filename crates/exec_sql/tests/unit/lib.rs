use super::*;
use crate::error::Redactor;
use crate::policy::{apply_masks, binds_for};
use crate::pool::{build_pool, mysql_write_grants};

use forge_core::{AccessMode, ColumnMask, DbConnection, DbConnectionInput, Driver, SqlSpec};
use serde_json::{json, Value};
use uuid::Uuid;

fn spec(sql: &str, params: Value) -> SqlSpec {
    serde_json::from_value(json!({
        "connection_id": Uuid::nil(),
        "mode": { "type": "query", "sql": sql, "params": params }
    }))
    .unwrap()
}

fn conn_input(driver: &str, mode: &str, ddl: bool) -> DbConnectionInput {
    serde_json::from_value(json!({
        "name": "db", "driver": driver, "dsn_ref": "env:DB",
        "access_mode": mode, "allow_ddl": ddl
    }))
    .unwrap()
}

#[test]
fn save_time_check() {
    let s = spec(
        "SELECT * FROM users WHERE id = :id",
        json!([{ "name": "id", "schema": { "type": "integer" } }]),
    );
    check_query_tool(&s, &conn_input("postgres", "read_only", false)).unwrap();

    let s = spec(
        "SELECT * FROM users WHERE id = :id",
        json!([{ "name": "other" }]),
    );
    let e = check_query_tool(&s, &conn_input("sqlite", "read_only", false)).unwrap_err();
    assert!(
        e.has("kind.mode.params") && e.has("kind.mode.params[0].name"),
        "{e}"
    );

    let s = spec("DELETE FROM users WHERE id = 1", json!([]));
    let e = check_query_tool(&s, &conn_input("mysql", "read_only", false)).unwrap_err();
    assert!(e.to_string().contains("read-only"), "{e}");
    check_query_tool(&s, &conn_input("mysql", "curated_write", false)).unwrap();

    let s = spec("DELETE FROM users", json!([]));
    let e = check_query_tool(&s, &conn_input("mysql", "full_access", false)).unwrap_err();
    assert!(e.to_string().contains("WHERE"), "{e}");

    let ddl = spec("CREATE INDEX i ON users (email)", json!([]));
    let e = check_query_tool(&ddl, &conn_input("sqlite", "curated_write", false)).unwrap_err();
    assert!(e.to_string().contains("schema changes"), "{e}");
    check_query_tool(&ddl, &conn_input("sqlite", "curated_write", true)).unwrap();

    let admin = spec("GRANT SELECT ON users TO bob", json!([]));
    let e = check_query_tool(&admin, &conn_input("postgres", "full_access", true)).unwrap_err();
    assert!(e.to_string().contains("never allowed"), "{e}");
}

#[test]
fn gating_matrix() {
    let q = |sql| guard::check_statement(sql, Driver::Postgres).unwrap();
    let (read, write, ddl) = (
        q("SELECT 1"),
        q("UPDATE t SET a = 1 WHERE id = 1"),
        q("DROP TABLE t"),
    );
    use AccessMode::*;
    for (mode, agent, w_ok) in [
        (ReadOnly, false, false),
        (ReadOnly, true, false),
        (CuratedWrite, false, true),
        (CuratedWrite, true, false),
        (FullAccess, false, true),
        (FullAccess, true, true),
    ] {
        assert!(gate(&read, mode, false, agent).is_ok());
        assert_eq!(
            gate(&write, mode, false, agent).is_ok(),
            w_ok,
            "{mode:?} agent={agent}"
        );
        assert!(gate(&ddl, mode, false, agent).is_err());
        assert_eq!(gate(&ddl, mode, true, agent).is_ok(), w_ok);
    }
}

#[test]
fn binds_follow_declared_types() {
    let s = spec(
        "SELECT :i, :n, :b, :s, :missing",
        json!([
            { "name": "i", "schema": { "type": "integer" } },
            { "name": "n", "schema": { "type": "number" } },
            { "name": "b", "schema": { "type": "boolean" } },
            { "name": "s" },
            { "name": "missing", "schema": { "type": "integer" } }
        ]),
    );
    let args = json!({ "i": 3.0, "n": 1.5, "b": true, "s": "x" });
    let binds = binds_for(
        &s,
        &["i", "n", "b", "s", "missing"].map(String::from),
        args.as_object().unwrap(),
    )
    .unwrap();
    assert_eq!(
        binds,
        [
            Bind::Int(3),
            Bind::Float(1.5),
            Bind::Bool(true),
            Bind::Text("x".into()),
            Bind::Null("integer")
        ]
    );
}

#[test]
fn masks_replace_values() {
    let masks = [
        ColumnMask::parse("users.password_hash").unwrap(),
        ColumnMask::parse("*.email").unwrap(),
    ];
    let cols = ["id", "email", "password_hash"].map(String::from);
    let mut rows = vec![
        vec![json!(1), json!("a@x.io"), json!("h")],
        vec![json!(2), Value::Null, json!("h2")],
    ];
    let hit = apply_masks(&masks, &["users".into()], &cols, &mut rows);
    assert_eq!(hit, ["email", "password_hash"]);
    assert_eq!(rows[0], [json!(1), json!(MASKED), json!(MASKED)]);
    assert_eq!(rows[1], [json!(2), Value::Null, json!(MASKED)]);
    // Table-scoped masks don't apply to other tables.
    let mut rows = vec![vec![json!(1), json!("e"), json!("h")]];
    let hit = apply_masks(&masks, &["orders".into()], &cols, &mut rows);
    assert_eq!(hit, ["email"]);
}

#[test]
fn redacts_dsn_and_password() {
    let r = Redactor::new("postgres://app:s3cr%40t@db.internal:5432/shop");
    let m = r.apply("failed for postgres://app:s3cr%40t@db.internal:5432/shop (pw s3cr@t)");
    assert!(!m.contains("s3cr"), "{m}");
    let r = Redactor::new("host=db user=app password=hunter22 dbname=shop");
    assert!(!r.apply("bad password hunter22").contains("hunter22"));
}

#[test]
fn network_drivers_need_url_dsns() {
    let conn: DbConnection = serde_json::from_value(json!({
        "id": Uuid::nil(), "project_id": Uuid::nil(), "name": "db", "driver": "postgres",
        "dsn_ref": "env:DB", "access_mode": "read_only", "row_limit": 1, "timeout_ms": 1000,
        "max_affected_rows": 1, "allow_ddl": false, "table_allowlist": [], "column_masks": [],
        "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z"
    }))
    .unwrap();
    let e = build_pool(&conn, r"C:\data\shop.db", false).unwrap_err();
    assert!(e.to_string().contains("postgres://"), "{e}");
}

#[test]
fn mysql_grant_parsing() {
    let (p, roles) = mysql_write_grants(&[
        "GRANT USAGE ON *.* TO `ro`@`%`".into(),
        "GRANT SELECT ON `shop`.* TO `ro`@`%`".into(),
    ]);
    assert!(p.is_empty() && !roles);
    let (p, roles) = mysql_write_grants(&[
        "GRANT SELECT, INSERT, UPDATE ON `shop`.* TO `app`@`%`".into(),
        "GRANT `writer`@`%` TO `app`@`%`".into(),
    ]);
    assert_eq!(p, ["INSERT ON `shop`.*", "UPDATE ON `shop`.*"]);
    assert!(roles);
    let (p, _) =
        mysql_write_grants(&["GRANT ALL PRIVILEGES ON *.* TO `root`@`%` WITH GRANT OPTION".into()]);
    assert_eq!(p.len(), 1);
}
