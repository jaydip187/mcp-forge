use std::sync::Arc;

use forge_core::{AuthConfig, HttpSpec, ToolInput, ToolKind};
use forge_exec_http::{ExecError, HttpExecutor, MapSecretResolver};
use helpers::*;
use serde_json::{json, Map, Value};
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Builds an `AuthConfig` from JSON so this test crate needs no chrono/uuid deps.
mod helpers {
    use serde_json::{json, Value};

    pub fn auth_config(kind: Value, secret_ref: &str) -> forge_core::AuthConfig {
        serde_json::from_value(json!({
            "id": "00000000-0000-0000-0000-000000000001",
            "project_id": "00000000-0000-0000-0000-000000000002",
            "name": "test-auth",
            "kind": kind,
            "secret_ref": secret_ref,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        }))
        .unwrap()
    }
}

fn spec(base: &str, v: Value) -> HttpSpec {
    let mut kind = v;
    kind["type"] = json!("http");
    kind["base_url"] = json!(base);
    let t: ToolInput = serde_json::from_value(json!({ "name": "t", "kind": kind })).unwrap();
    t.validate().unwrap();
    let ToolKind::Http(h) = t.kind;
    h
}

fn args(v: Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}

fn exec(secrets: MapSecretResolver) -> HttpExecutor {
    HttpExecutor::new(Arc::new(secrets))
}

#[tokio::test]
async fn get_with_path_query_header_and_fixed_values() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/users/a%20b"))
        .and(query_param("fields", "name"))
        .and(query_param("tag", "x"))
        .and(header("X-Version", "2"))
        .and(header("X-Trace", "abc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "a b" })))
        .expect(1)
        .mount(&server)
        .await;

    let s = spec(
        &format!("{}/v1/", server.uri()),
        json!({
            "method": "GET",
            "path_template": "/users/{id}",
            "params": [
                { "name": "id", "location": "path", "required": true },
                { "name": "fields", "location": "query" },
                { "name": "tag", "location": "query", "schema": { "type": "array" } },
                { "name": "X-Trace", "location": "header" },
                { "name": "X-Version", "location": "header", "fixed_value": "2" }
            ]
        }),
    );
    let out = exec(MapSecretResolver::default())
        .execute(
            &s,
            None,
            &args(json!({ "id": "a b", "fields": "name", "tag": ["x"], "X-Trace": "abc" })),
        )
        .await
        .unwrap();
    assert_eq!(out.status, 200);
    assert!(out.is_success());
    assert_eq!(
        serde_json::from_str::<Value>(&out.body).unwrap(),
        json!({ "id": "a b" })
    );
    assert!(out.content_type.unwrap().starts_with("application/json"));
}

#[tokio::test]
async fn post_body_params_and_whole_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/items"))
        .and(body_json(json!({ "title": "t", "count": 2 })))
        .respond_with(ResponseTemplate::new(201).set_body_string("created"))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/raw"))
        .and(body_json(json!({ "a": [1, 2] })))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let e = exec(MapSecretResolver::default());
    let s = spec(
        &server.uri(),
        json!({
            "method": "POST",
            "path_template": "/items",
            "params": [
                { "name": "title", "location": "body", "required": true },
                { "name": "count", "location": "body", "schema": { "type": "integer" } }
            ]
        }),
    );
    let out = e
        .execute(&s, None, &args(json!({ "title": "t", "count": 2 })))
        .await
        .unwrap();
    assert_eq!((out.status, out.body.as_str()), (201, "created"));

    let s = spec(
        &server.uri(),
        json!({ "method": "PUT", "path_template": "/raw", "body_schema": { "type": "object" } }),
    );
    let out = e
        .execute(&s, None, &args(json!({ "body": { "a": [1, 2] } })))
        .await
        .unwrap();
    assert_eq!(out.status, 204);
}

#[tokio::test]
async fn auth_kinds_applied_and_redacted() {
    let server = MockServer::start().await;
    Mock::given(path("/bearer"))
        .and(header("Authorization", "Bearer tok-123"))
        .respond_with(ResponseTemplate::new(200).set_body_string("echo tok-123"))
        .mount(&server)
        .await;
    Mock::given(path("/basic"))
        .and(header("Authorization", "Basic dXNlcjpwdw=="))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    Mock::given(path("/qkey"))
        .and(query_param("api_key", "k+1"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    Mock::given(path("/hkey"))
        .and(header("X-Key", "k+1"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let e = exec(
        MapSecretResolver::default()
            .with("env:TOK", "tok-123")
            .with("env:PW", "pw")
            .with("env:KEY", "k+1"),
    );
    let get = |p: &str| {
        spec(
            &server.uri(),
            json!({ "method": "GET", "path_template": p }),
        )
    };
    let none = Map::new();

    let a = auth_config(json!({ "type": "bearer" }), "env:TOK");
    let out = e.execute(&get("/bearer"), Some(&a), &none).await.unwrap();
    assert_eq!(out.status, 200);
    assert_eq!(
        out.body, "echo [REDACTED]",
        "secret echoed upstream must be redacted"
    );

    let a = auth_config(json!({ "type": "basic", "username": "user" }), "env:PW");
    assert_eq!(
        e.execute(&get("/basic"), Some(&a), &none)
            .await
            .unwrap()
            .status,
        200
    );

    let a = auth_config(
        json!({ "type": "api_key", "location": "query", "name": "api_key" }),
        "env:KEY",
    );
    let out = e.execute(&get("/qkey"), Some(&a), &none).await.unwrap();
    assert_eq!(out.status, 200);
    assert!(
        !out.url.contains("k+1") && !out.url.contains("k%2B1"),
        "{}",
        out.url
    );
    assert!(out.url.contains("api_key=[REDACTED]"), "{}", out.url);

    let a = auth_config(
        json!({ "type": "api_key", "location": "header", "name": "X-Key" }),
        "env:KEY",
    );
    assert_eq!(
        e.execute(&get("/hkey"), Some(&a), &none)
            .await
            .unwrap()
            .status,
        200
    );
}

#[tokio::test]
async fn missing_secret_is_config_error_without_value() {
    let e = exec(MapSecretResolver::default());
    let s = spec(
        "http://127.0.0.1:9",
        json!({ "method": "GET", "path_template": "/" }),
    );
    let a: AuthConfig = auth_config(json!({ "type": "bearer" }), "env:NOPE");
    let err = e.execute(&s, Some(&a), &Map::new()).await.unwrap_err();
    assert!(
        matches!(&err, ExecError::Config(m) if m.contains("env:NOPE")),
        "{err}"
    );
}

#[tokio::test]
async fn upstream_errors_are_outcomes() {
    let server = MockServer::start().await;
    Mock::given(path("/missing"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({ "error": "no such user" })))
        .mount(&server)
        .await;
    let s = spec(
        &server.uri(),
        json!({ "method": "GET", "path_template": "/missing" }),
    );
    let out = exec(MapSecretResolver::default())
        .execute(&s, None, &Map::new())
        .await
        .unwrap();
    assert!(!out.is_success());
    assert_eq!(out.status_line(), "404 Not Found");
    assert!(out.body.contains("no such user"));
}

#[tokio::test]
async fn invalid_args_name_the_argument() {
    let s = spec(
        "http://127.0.0.1:9",
        json!({
            "method": "GET",
            "path_template": "/u/{id}",
            "params": [{ "name": "id", "location": "path", "required": true, "schema": { "type": "integer" } }]
        }),
    );
    let e = exec(MapSecretResolver::default());
    let err = e
        .execute(&s, None, &args(json!({ "id": "x", "bogus": 1 })))
        .await
        .unwrap_err();
    let ExecError::InvalidArgs(v) = err else {
        panic!("expected InvalidArgs")
    };
    assert!(v.has("id") && v.has("bogus"), "{v}");
}

#[tokio::test]
async fn timeout_and_connect_errors() {
    let server = MockServer::start().await;
    Mock::given(path("/slow"))
        .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_millis(500)))
        .mount(&server)
        .await;
    let mut s = spec(
        &server.uri(),
        json!({ "method": "GET", "path_template": "/slow" }),
    );
    s.timeout_ms = 50;
    let e = exec(MapSecretResolver::default());
    let err = e.execute(&s, None, &Map::new()).await.unwrap_err();
    assert!(
        matches!(&err, ExecError::Request(m) if m.contains("timed out after 50 ms")),
        "{err}"
    );

    // Port 9 (discard) is essentially never listening on loopback.
    let s = spec(
        "http://127.0.0.1:9",
        json!({ "method": "GET", "path_template": "/" }),
    );
    let err = e.execute(&s, None, &Map::new()).await.unwrap_err();
    assert!(matches!(err, ExecError::Request(_)), "{err}");
}

#[tokio::test]
async fn cross_host_redirects_not_followed() {
    let server = MockServer::start().await;
    Mock::given(path("/hop"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("Location", "http://localhost:9/steal"),
        )
        .mount(&server)
        .await;
    let s = spec(
        &server.uri(),
        json!({ "method": "GET", "path_template": "/hop" }),
    );
    let out = exec(MapSecretResolver::default())
        .execute(&s, None, &Map::new())
        .await
        .unwrap();
    assert_eq!(out.status, 302);
}
