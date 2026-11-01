use forge_core::ColumnMask;
use serde_json::json;

use super::*;

#[test]
fn globs() {
    assert!(glob("user:*", "user:42"));
    assert!(glob("logs-202?-*", "logs-2026-11"));
    assert!(!glob("user:*", "admin:1"));
    assert!(glob("*", ""));
}

#[test]
fn gates() {
    use forge_core::AccessMode::*;
    assert!(gate(OpClass::Read, ReadOnly, true).is_ok());
    assert!(gate(OpClass::Write, ReadOnly, false).is_err());
    assert!(gate(OpClass::Write, CuratedWrite, false).is_ok());
    assert!(
        gate(OpClass::Write, CuratedWrite, true).is_err(),
        "agents need full access"
    );
    assert!(gate(OpClass::Write, FullAccess, true).is_ok());
    assert!(class_of(Driver::Redis, "FLUSHALL")
        .unwrap_err()
        .contains("GET"));
}

#[test]
fn targets() {
    let allow = vec!["users".to_owned(), "orders_*".to_owned()];
    assert!(check_target(Driver::Mongodb, "users", &allow).is_ok());
    assert!(check_target(Driver::Mongodb, "orders_2026", &allow).is_ok());
    assert!(check_target(Driver::Mongodb, "secrets", &allow).is_err());
    assert!(check_target(Driver::Mongodb, "system.users", &[]).is_err());
    assert!(check_target(Driver::Elasticsearch, ".security", &[]).is_err());
    assert!(check_target(Driver::Elasticsearch, "a,b", &[]).is_err());
    assert!(
        check_target(Driver::Elasticsearch, "logs-*", &[]).is_err(),
        "wildcards need allowing"
    );
    assert!(check_target(Driver::Elasticsearch, "logs-*", &["logs-*".into()]).is_ok());
    assert!(check_target(Driver::Redis, "session:abc", &["user:*".into()]).is_err());
}

#[test]
fn bodies() {
    let allow = vec!["users".to_owned(), "orders".to_owned()];
    for bad in [
        json!({ "filter": { "$where": "this.a > 1" } }),
        json!({ "pipeline": [{ "$out": "copy" }] }),
        json!({ "pipeline": [{ "$merge": { "into": "x" } }] }),
        json!({ "pipeline": [{ "$group": { "_id": 1, "x": { "$accumulator": {} } } }] }),
        json!({ "pipeline": [{ "$lookup": { "from": "secrets", "localField": "a", "foreignField": "b", "as": "c" } }] }),
        json!({ "pipeline": [{ "$unionWith": "secrets" }] }),
    ] {
        assert!(check_body(Driver::Mongodb, &bad, &allow).is_err(), "{bad}");
    }
    let ok = json!({ "pipeline": [{ "$lookup": { "from": "orders", "localField": "id", "foreignField": "user", "as": "o" } }] });
    assert!(check_body(Driver::Mongodb, &ok, &allow).is_ok());
    assert!(check_body(
        Driver::Elasticsearch,
        &json!({ "query": { "script": { "script": "1" } } }),
        &[]
    )
    .is_err());
    assert!(check_body(
        Driver::Elasticsearch,
        &json!({ "aggs": { "x": { "scripted_metric": {} } } }),
        &[]
    )
    .is_err());
    assert!(check_body(
        Driver::Elasticsearch,
        &json!({ "query": { "match": { "title": "x" } } }),
        &[]
    )
    .is_ok());
    assert!(check_body(Driver::Dynamodb, &json!({ "TableName": "other" }), &[]).is_err());
    assert!(check_body(Driver::Redis, &json!(["a", 1]), &[]).is_ok());
    assert!(check_body(Driver::Redis, &json!([{ "x": 1 }]), &[]).is_err());
}

#[test]
fn masks() {
    let masks = vec![
        ColumnMask::parse("users.password").unwrap(),
        ColumnMask::parse("*.ssn").unwrap(),
    ];
    let mut docs = vec![
        json!({ "name": "ada", "password": "h", "profile": { "ssn": "1", "Password": null } }),
    ];
    let hit = apply_masks(&masks, "users", &mut docs);
    assert_eq!(docs[0]["password"], json!(MASKED));
    assert_eq!(docs[0]["profile"]["ssn"], json!(MASKED));
    assert_eq!(
        docs[0]["profile"]["Password"],
        json!(null),
        "nulls stay null"
    );
    assert!(hit.contains(&"password".to_owned()) && hit.contains(&"ssn".to_owned()));
    let mut other = vec![json!({ "password": "x" })];
    apply_masks(&masks, "orders", &mut other);
    assert_eq!(
        other[0]["password"],
        json!("x"),
        "users.password only covers users"
    );

    assert!(check_masks(&masks, "users", &json!({ "filter": { "password": "x" } })).is_err());
    assert!(check_masks(
        &masks,
        "users",
        &json!({ "pipeline": [{ "$project": { "p": "$password" } }] })
    )
    .is_err());
    assert!(check_masks(&masks, "users", &json!({ "filter": { "profile.ssn": 1 } })).is_err());
    assert!(
        check_masks(
            &masks,
            "users",
            &json!({ "filter": { "name": "password" } })
        )
        .is_ok(),
        "a plain value is fine"
    );
}
