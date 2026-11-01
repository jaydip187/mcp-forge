//! MongoDB and Redis in Docker (testcontainers). Run with
//! `cargo test -p forge-exec-nosql --test containers -- --ignored`
//! (needs a running Docker daemon).

use std::collections::HashMap;
use std::sync::Arc;

use forge_core::{DbConnection, NosqlSpec, SecretRef, SecretResolver};
use forge_exec_nosql::NosqlExecutor;
use forge_exec_sql::{SqlError, MASKED};
use serde_json::{json, Map, Value};
use testcontainers_modules::mongo::Mongo;
use testcontainers_modules::redis::Redis;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use uuid::Uuid;

struct Secrets(HashMap<String, String>);

impl SecretResolver for Secrets {
    fn resolve(&self, r: &SecretRef) -> Result<String, String> {
        self.0
            .get(r.name())
            .cloned()
            .ok_or_else(|| "missing".into())
    }
}

fn setup(driver: &str, dsn: &str, extra: Value) -> (NosqlExecutor, DbConnection) {
    let exec = NosqlExecutor::new(Arc::new(Secrets(HashMap::from([(
        "DB".to_owned(),
        dsn.to_owned(),
    )]))));
    let mut v = json!({
        "id": Uuid::new_v4(), "project_id": Uuid::new_v4(), "name": "db", "driver": driver,
        "dsn_ref": "env:DB", "access_mode": "curated_write", "row_limit": 2, "timeout_ms": 3000,
        "max_affected_rows": 1, "allow_ddl": false, "table_allowlist": [], "column_masks": [],
        "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z"
    });
    for (k, x) in extra.as_object().unwrap() {
        v[k] = x.clone();
    }
    (exec, serde_json::from_value(v).unwrap())
}

fn spec(op: &str, target: &str, body: Value, params: Value) -> NosqlSpec {
    serde_json::from_value(json!({
        "connection_id": Uuid::nil(), "operation": op, "target": target, "body": body, "params": params
    }))
    .unwrap()
}

fn args(v: Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}

#[tokio::test]
#[ignore = "needs Docker"]
async fn mongodb() {
    let node = Mongo::default().start().await.unwrap();
    let port = node.get_host_port_ipv4(27017).await.unwrap();
    let (exec, conn) = setup(
        "mongodb",
        &format!("mongodb://127.0.0.1:{port}/shop"),
        json!({ "column_masks": ["users.password"] }),
    );
    for (name, pw) in [("ada", "h1"), ("bob", "h2"), ("cy", "h3")] {
        let ins = spec(
            "insert_one",
            "users",
            json!({ "document": { "name": name, "password": pw, "age": 30 } }),
            json!([]),
        );
        exec.run(&conn, &ins, &args(json!({})))
            .await
            .result
            .unwrap();
    }
    let find = spec(
        "find",
        "users",
        json!({ "filter": { "age": { "$gte": "{{min}}" } }, "sort": { "name": 1 } }),
        json!([{ "name": "min", "schema": { "type": "integer" } }]),
    );
    let out = exec
        .run(&conn, &find, &args(json!({ "min": 18 })))
        .await
        .result
        .unwrap();
    assert_eq!(out.documents.len(), 2, "row limit");
    assert!(out.truncated);
    assert_eq!(out.documents[0]["name"], "ada");
    assert_eq!(out.documents[0]["password"], MASKED);
    // `$`-strings can't become field paths.
    let e = exec
        .run(&conn, &find, &args(json!({ "min": "$password" })))
        .await
        .result
        .unwrap_err();
    assert!(
        matches!(e, SqlError::Rejected(_) | SqlError::InvalidArgs(_)),
        "{e}"
    );
    // The write cap counts first.
    let upd = spec(
        "update_many",
        "users",
        json!({ "filter": {}, "update": { "$set": { "age": 1 } } }),
        json!([]),
    );
    let e = exec
        .run(&conn, &upd, &args(json!({})))
        .await
        .result
        .unwrap_err();
    assert!(matches!(e, SqlError::TooManyRows { .. }), "{e}");
    let agg = spec(
        "aggregate",
        "users",
        json!({ "pipeline": [{ "$out": "copy" }] }),
        json!([]),
    );
    assert!(exec
        .run(&conn, &agg, &args(json!({})))
        .await
        .result
        .is_err());
    let report = exec.test_connection(&conn).await;
    assert!(report.ok, "{report:?}");
}

#[tokio::test]
#[ignore = "needs Docker"]
async fn redis() {
    let node = Redis::default().start().await.unwrap();
    let port = node.get_host_port_ipv4(6379).await.unwrap();
    let (exec, conn) = setup(
        "redis",
        &format!("redis://127.0.0.1:{port}/0"),
        json!({ "table_allowlist": ["user:*"], "max_affected_rows": 5 }),
    );
    let hset = spec(
        "HSET",
        "user:{{id}}",
        json!(["name", "ada", "password", "h1"]),
        json!([{ "name": "id" }]),
    );
    exec.run(&conn, &hset, &args(json!({ "id": "1" })))
        .await
        .result
        .unwrap();
    for i in 0..5 {
        let push = spec(
            "RPUSH",
            "user:1:events",
            json!([format!("e{i}")]),
            json!([]),
        );
        exec.run(&conn, &push, &args(json!({})))
            .await
            .result
            .unwrap();
    }
    let get = spec(
        "HGETALL",
        "user:{{id}}",
        Value::Null,
        json!([{ "name": "id" }]),
    );
    let out = exec
        .run(&conn, &get, &args(json!({ "id": "1" })))
        .await
        .result
        .unwrap();
    assert_eq!(out.documents[0]["name"], "ada");
    let range = spec("LRANGE", "user:1:events", json!([0, -1]), json!([]));
    let out = exec
        .run(&conn, &range, &args(json!({})))
        .await
        .result
        .unwrap();
    assert_eq!(out.documents.len(), 2);
    assert!(out.truncated);
    let other = spec("GET", "session:1", Value::Null, json!([]));
    assert!(
        exec.run(&conn, &other, &args(json!({})))
            .await
            .result
            .is_err(),
        "outside the allowlist"
    );
    let report = exec.test_connection(&conn).await;
    assert!(report.ok, "{report:?}");
}
