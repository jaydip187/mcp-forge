//! Executor runs against mocked Elasticsearch and DynamoDB endpoints, plus
//! DynamoDB value conversion and SigV4.

use std::collections::HashMap;

use forge_core::{DbConnection, SecretRef};
use serde_json::json;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

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
        "id": Uuid::new_v4(), "project_id": Uuid::new_v4(), "name": "db",
        "driver": driver, "dsn_ref": "env:DB", "access_mode": "read_only",
        "row_limit": 2, "timeout_ms": 2000, "max_affected_rows": 5, "allow_ddl": false,
        "table_allowlist": [], "column_masks": [],
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
async fn elasticsearch_search_caps_masks_and_refuses_scripts() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/products/_search"))
        .and(body_partial_json(
            json!({ "size": 3, "query": { "match": { "title": "lamp" } } }),
        ))
        .and(header("Authorization", "ApiKey k3y"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "hits": { "total": { "value": 7 }, "hits": [
                { "_id": "1", "_index": "products", "_source": { "title": "lamp a", "cost": 3 } },
                { "_id": "2", "_index": "products", "_source": { "title": "lamp b", "cost": 4 } },
                { "_id": "3", "_index": "products", "_source": { "title": "lamp c", "cost": 5 } }
            ] }
        })))
        .expect(1)
        .mount(&server)
        .await;
    let (exec, conn) = setup(
        "elasticsearch",
        &format!("{}?api_key=k3y", server.uri()),
        json!({ "column_masks": ["products.cost"] }),
    );
    let s = spec(
        "search",
        "products",
        json!({ "query": { "match": { "title": "{{q}}" } }, "size": 50 }),
        json!([{ "name": "q" }]),
    );
    let out = exec
        .run(&conn, &s, &args(json!({ "q": "lamp" })))
        .await
        .result
        .unwrap();
    assert_eq!(out.documents.len(), 2);
    assert!(out.truncated);
    assert_eq!(
        out.documents[0],
        json!({ "_id": "1", "title": "lamp a", "cost": MASKED_STR })
    );
    assert_eq!(out.result_json()["total"], json!(7));

    let bad = spec(
        "search",
        "products",
        json!({ "query": { "script": { "script": "1" } } }),
        json!([]),
    );
    let e = exec
        .run(&conn, &bad, &args(json!({})))
        .await
        .result
        .unwrap_err();
    assert!(matches!(e, SqlError::Rejected(_)), "{e}");
    // Read-only connection: writes are refused before any request.
    let w = spec("delete", "products", json!({ "id": "1" }), json!([]));
    let e = exec
        .run(&conn, &w, &args(json!({})))
        .await
        .result
        .unwrap_err();
    assert!(e.to_string().contains("read-only"), "{e}");
}

const MASKED_STR: &str = forge_exec_sql::MASKED;

#[tokio::test]
async fn elasticsearch_ids_are_path_segments_and_writes_audited() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/products/_doc/a%2F..%2Fb"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "_id": "a/../b", "result": "deleted" })),
        )
        .expect(1)
        .mount(&server)
        .await;
    let (exec, conn) = setup(
        "elasticsearch",
        &server.uri(),
        json!({ "access_mode": "curated_write" }),
    );
    let s = spec(
        "delete",
        "products",
        json!({ "id": "{{id}}" }),
        json!([{ "name": "id" }]),
    );
    let dry = exec
        .run(&conn, &s, &args(json!({ "id": "a/../b", "dry_run": true })))
        .await;
    assert_eq!(dry.audit.unwrap().outcome, AuditOutcome::DryRun);
    let run = exec.run(&conn, &s, &args(json!({ "id": "a/../b" }))).await;
    let out = run.result.unwrap();
    assert_eq!(out.write.unwrap().affected_rows, 1);
    let audit = run.audit.unwrap();
    assert_eq!(audit.outcome, AuditOutcome::Committed);
    assert_eq!(audit.params, [json!("a/../b")]);
}

#[tokio::test]
async fn dynamodb_query_is_signed_marshalled_and_capped() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/"))
        .and(header("x-amz-target", "DynamoDB_20120810.Query"))
        .and(body_partial_json(json!({
            "TableName": "orders",
            "Limit": 3,
            "KeyConditionExpression": "customer = :c",
            "ExpressionAttributeValues": { ":c": { "S": "ada" }, ":n": { "N": "5" } }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "Items": [
                { "customer": { "S": "ada" }, "total": { "N": "12.5" }, "tags": { "L": [{ "S": "a" }] } },
                { "customer": { "S": "ada" }, "total": { "N": "3" }, "paid": { "BOOL": true } }
            ],
            "Count": 2
        })))
        .expect(1)
        .mount(&server)
        .await;
    let dsn = format!(
        "dynamodb://AKIDEXAMPLE:wJalr%2Fsecret@us-east-1?endpoint={}",
        server.uri()
    );
    let (exec, conn) = setup("dynamodb", &dsn, json!({}));
    let s = spec(
        "Query",
        "orders",
        json!({ "KeyConditionExpression": "customer = :c", "ExpressionAttributeValues": { ":c": "{{c}}", ":n": { "N": "{{n}}" } } }),
        json!([{ "name": "c" }, { "name": "n", "schema": { "type": "integer" } }]),
    );
    let out = exec
        .run(&conn, &s, &args(json!({ "c": "ada", "n": 5 })))
        .await
        .result
        .unwrap();
    assert_eq!(
        out.documents,
        [
            json!({ "customer": "ada", "total": 12.5, "tags": ["a"] }),
            json!({ "customer": "ada", "total": 3, "paid": true })
        ]
    );
    let requests = server.received_requests().await.unwrap();
    let auth = requests[0]
        .headers
        .get("authorization")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        auth.starts_with("AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/"),
        "{auth}"
    );
    assert!(auth.contains("/us-east-1/dynamodb/aws4_request"));
    assert!(!auth.contains("secret"));
}

#[tokio::test]
async fn dynamodb_errors_are_redacted_and_classified() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "__type": "com.amazonaws.dynamodb.v20120810#ResourceNotFoundException",
            "message": "Requested resource not found"
        })))
        .mount(&server)
        .await;
    let dsn = format!(
        "dynamodb://AK:s3cretKEY@eu-west-1?endpoint={}",
        server.uri()
    );
    let (exec, conn) = setup("dynamodb", &dsn, json!({ "table_allowlist": ["orders"] }));
    let e = exec
        .run(
            &conn,
            &spec(
                "GetItem",
                "orders",
                json!({ "Key": { "id": 1 } }),
                json!([]),
            ),
            &args(json!({})),
        )
        .await
        .result
        .unwrap_err();
    assert_eq!(
        e,
        SqlError::Database("ResourceNotFoundException: Requested resource not found".into())
    );
    let e = exec
        .run(
            &conn,
            &spec("GetItem", "users", json!({}), json!([])),
            &args(json!({})),
        )
        .await
        .result
        .unwrap_err();
    assert!(e.to_string().contains("allowed: orders"), "{e}");
}

#[test]
fn dynamodb_values() {
    assert_eq!(dynamo::marshal(&json!(5)), json!({ "N": "5" }));
    assert_eq!(
        dynamo::marshal(&json!({ "N": 5 })),
        json!({ "N": "5" }),
        "typed values pass, N as text"
    );
    assert_eq!(
        dynamo::marshal(&json!({ "a": [true, null] })),
        json!({ "M": { "a": { "L": [{ "BOOL": true }, { "NULL": true }] } } })
    );
    assert_eq!(
        dynamo::unmarshal(&json!({ "NS": ["1", "2.5"] })),
        json!([1, 2.5])
    );
    let req = dynamo::prepare("t", &json!({ "Key": { "id": "x" } }));
    assert_eq!(
        Value::Object(req),
        json!({ "TableName": "t", "Key": { "id": { "S": "x" } } })
    );
}

#[test]
fn sigv4_signing_key_matches_aws_example() {
    // From the AWS Signature Version 4 documentation.
    let k = dynamo::signing_key(
        "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
        "20120215",
        "us-east-1",
        "iam",
    );
    assert_eq!(
        hex::encode(k),
        "f4780e2d9f65fa895f9c67b32ce1baf0b0d8a43505a000a1a9e090d414db404d"
    );
}

#[tokio::test]
async fn exploration_tools() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/_cat/indices"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "index": "products" }, { "index": ".kibana" }, { "index": "orders" }, { "index": "secret" }
        ])))
        .mount(&server)
        .await;
    let (exec, conn) = setup(
        "elasticsearch",
        &server.uri(),
        json!({ "table_allowlist": ["products", "orders"], "access_mode": "curated_write" }),
    );
    let out = exec
        .run(
            &conn,
            &spec(LIST_TARGETS, "", Value::Null, json!([])),
            &args(json!({})),
        )
        .await
        .result
        .unwrap();
    assert_eq!(
        out.documents,
        [json!({ "name": "orders" }), json!({ "name": "products" })]
    );
    // run_query is read-only under saved writes, and checks the target.
    let rq = spec(RUN_QUERY, "", Value::Null, json!([]));
    let e = exec
        .run(
            &conn,
            &rq,
            &args(json!({ "operation": "delete", "target": "products", "body": { "id": "1" } })),
        )
        .await
        .result
        .unwrap_err();
    assert!(e.to_string().contains("read-only"), "{e}");
    let e = exec
        .run(
            &conn,
            &rq,
            &args(json!({ "operation": "search", "target": "secret", "body": "{}" })),
        )
        .await
        .result
        .unwrap_err();
    assert!(e.to_string().contains("isn't available"), "{e}");
    assert!(!tool_writes(&rq, &conn));
}
