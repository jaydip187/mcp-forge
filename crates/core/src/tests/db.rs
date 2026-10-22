use super::*;

fn conn(v: Value) -> DbConnectionInput {
    serde_json::from_value(v).unwrap()
}

#[test]
fn connection_defaults_and_rules() {
    let c = conn(json!({ "name": "main", "driver": "postgres", "dsn_ref": "env:DB" }));
    c.validate().unwrap();
    assert_eq!(c.access_mode, AccessMode::ReadOnly);
    assert_eq!(c.row_limit, DEFAULT_ROW_LIMIT);
    assert_eq!(c.timeout_ms, DEFAULT_SQL_TIMEOUT_MS);

    let c = conn(json!({
        "name": "bad name", "driver": "mysql", "dsn_ref": "env:DB",
        "allow_ddl": true, "row_limit": 0, "timeout_ms": 5,
        "table_allowlist": ["ok", "no spaces"]
    }));
    let e = c.validate().unwrap_err();
    for f in [
        "name",
        "allow_ddl",
        "row_limit",
        "timeout_ms",
        "table_allowlist[1]",
    ] {
        assert!(e.has(f), "{f}: {e}");
    }
    // Raw DSNs never deserialize.
    assert!(serde_json::from_value::<DbConnectionInput>(json!({
        "name": "x", "driver": "postgres", "dsn_ref": "postgres://u:pw@h/db"
    }))
    .is_err());
}

#[test]
fn column_masks() {
    let m = ColumnMask::parse("users.password_hash").unwrap();
    assert_eq!(m.to_string(), "users.password_hash");
    assert!(m.applies("PASSWORD_HASH", &["public.users".into()]));
    assert!(!m.applies("password_hash", &["orders".into()]));
    let any = ColumnMask::parse("*.email").unwrap();
    assert!(any.applies("email", &[]));
    let q = ColumnMask::parse("public.users.email").unwrap();
    assert_eq!(q.table, "public.users");
    assert!(q.applies("email", &["users".into()]));
    for bad in ["email", "users.", ".email", "a b.c", "t.c;"] {
        assert!(ColumnMask::parse(bad).is_err(), "{bad}");
    }
    assert_eq!(
        serde_json::to_value(&m).unwrap(),
        json!("users.password_hash")
    );
}

#[test]
fn sql_spec_schema_and_validation() {
    let s: SqlSpec = serde_json::from_value(json!({
            "connection_id": Uuid::nil(),
            "mode": { "type": "query", "sql": "SELECT * FROM users WHERE id = :id AND name LIKE :q",
                "params": [
                    { "name": "id", "schema": { "type": "integer" }, "required": true, "description": "User id" },
                    { "name": "q" }
                ] }
        }))
        .unwrap();
    let d = s.derive_input();
    assert_eq!(
        Value::Object(d.schema),
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "integer", "description": "User id" },
                "q": { "type": "string" }
            },
            "required": ["id"],
            "additionalProperties": false
        })
    );
    let mut errs = ValidationErrors::default();
    s.validate_into(&mut errs);
    assert!(errs.is_empty(), "{errs}");

    let bad: SqlSpec = serde_json::from_value(json!({
        "connection_id": Uuid::nil(),
        "mode": { "type": "query", "sql": " ", "params": [
            { "name": "1x" }, { "name": "a", "schema": { "type": "array" } }, { "name": "A" }
        ] }
    }))
    .unwrap();
    let mut errs = ValidationErrors::default();
    bad.validate_into(&mut errs);
    for f in [
        "kind.mode.sql",
        "kind.mode.params[0].name",
        "kind.mode.params[1].schema",
        "kind.mode.params[2].name",
    ] {
        assert!(errs.has(f), "{f}: {errs}");
    }
}

#[test]
fn exploration_modes_have_fixed_inputs() {
    let spec = |mode: Value| -> SqlSpec {
        serde_json::from_value(json!({ "connection_id": Uuid::nil(), "mode": mode })).unwrap()
    };
    let d = spec(json!({ "type": "run_sql" })).derive_input();
    assert_eq!(d.schema["required"], json!(["sql"]));
    assert_eq!(d.bindings[0].arg, "sql");
    let d = spec(json!({ "type": "list_tables" })).derive_input();
    assert_eq!(d.schema["required"], json!([]));
    assert!(d.schema["properties"]["pattern"].is_object());
    let s = spec(json!({ "type": "describe_table" }));
    assert_eq!(s.mode.as_str(), "describe_table");
    assert!(s.params().is_empty());
    let mut errs = ValidationErrors::default();
    s.validate_into(&mut errs);
    assert!(errs.is_empty());
}

#[test]
fn write_modes_and_ddl() {
    for mode in ["curated_write", "full_access"] {
        conn(json!({ "name": "w", "driver": "sqlite", "dsn_ref": "env:DB", "access_mode": mode, "allow_ddl": true }))
                .validate()
                .unwrap();
    }
}
