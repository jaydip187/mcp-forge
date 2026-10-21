//! Stage 3 backend: security guard, static UI shell, info, MCP view, test console.

use std::sync::Arc;

use forge_exec_http::MapSecretResolver;
use forge_store::Store;
use mcp_forge::{AppState, Security};
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use wiremock::matchers::{header, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "test-admin-token-123";

async fn start(security: Security) -> String {
    let store = Store::open_in_memory().await.unwrap();
    let secrets = MapSecretResolver::default().with("env:KEY", "key-value-xyz");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let state = AppState::new(store, Arc::new(secrets))
        .with_security(security)
        .with_data_dir("/data".into());
    tokio::spawn(mcp_forge::serve(listener, state, std::future::pending()));
    base
}

fn token_security() -> Security {
    Security {
        admin_token: Some(TOKEN.into()),
        loopback_hosts_only: false,
    }
}

#[tokio::test]
async fn loopback_mode_rejects_foreign_host_headers() {
    let base = start(Security::default()).await;
    let c = Client::new();
    for p in ["/api/projects", "/", "/mcp/x"] {
        let r = c
            .get(format!("{base}{p}"))
            .header("Host", "evil.example:7410")
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::FORBIDDEN, "{p}");
    }
    let ok = c.get(format!("{base}/api/projects")).send().await.unwrap();
    assert_eq!(ok.status(), StatusCode::OK);
    let ok = c
        .get(format!("{base}/api/projects"))
        .header("Host", "localhost:7410")
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK);
}

#[tokio::test]
async fn admin_token_guards_api_and_mcp_but_not_ui_shell() {
    let base = start(token_security()).await;
    let c = Client::new();

    let r = c.get(format!("{base}/api/projects")).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(r.headers()["www-authenticate"], "Bearer");
    let r = c
        .get(format!("{base}/api/projects"))
        .bearer_auth("wrong-token-wrong-token")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    let r = c
        .post(format!("{base}/mcp/demo"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);

    let r = c
        .get(format!("{base}/api/projects"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);

    // Foreign Host is fine here: the token is the protection.
    let r = c
        .get(format!("{base}/api/info"))
        .header("Host", "forge.internal:7410")
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let info: Value = r.json().await.unwrap();
    assert_eq!(info["auth_required"], true);

    for p in ["/", "/healthz"] {
        let r = c.get(format!("{base}{p}")).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::OK, "{p}");
    }
}

#[tokio::test]
async fn ui_shell_is_html_with_csp() {
    let base = start(Security::default()).await;
    let r = reqwest::get(format!("{base}/")).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert!(r.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/html"));
    let csp = r.headers()["content-security-policy"].to_str().unwrap();
    assert!(csp.contains("default-src 'self'") && csp.contains("frame-ancestors 'none'"));
    assert_eq!(r.headers()["x-content-type-options"], "nosniff");
    let missing = reqwest::get(format!("{base}/assets/nope.js"))
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn info_reports_snippet_inputs() {
    let base = start(Security::default()).await;
    let info: Value = reqwest::get(format!("{base}/api/info"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(info["auth_required"], false);
    assert_eq!(info["data_dir"], "/data");
    assert!(info["exe"].is_string());
    assert!(info["version"].is_string());
}

async fn post(c: &Client, url: String, body: Value) -> Value {
    let r = c.post(url).json(&body).send().await.unwrap();
    assert!(r.status().is_success(), "{}", r.text().await.unwrap());
    r.json().await.unwrap()
}

#[tokio::test]
async fn test_console_and_mcp_view() {
    let upstream = MockServer::start().await;
    Mock::given(path("/items/5"))
        .and(header("X-Key", "key-value-xyz"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "id": 5, "echo": "key-value-xyz" })),
        )
        .mount(&upstream)
        .await;

    let base = start(Security::default()).await;
    let c = Client::new();
    let p = post(
        &c,
        format!("{base}/api/projects"),
        json!({ "name": "Demo" }),
    )
    .await;
    let pid = p["id"].as_str().unwrap();
    let auth = post(
        &c,
        format!("{base}/api/projects/{pid}/auth-configs"),
        json!({ "name": "key", "kind": { "type": "api_key", "location": "header", "name": "X-Key" }, "secret_ref": "env:KEY" }),
    )
    .await;
    let missing_auth = post(
        &c,
        format!("{base}/api/projects/{pid}/auth-configs"),
        json!({ "name": "unset", "kind": { "type": "bearer" }, "secret_ref": "env:NOT_SET" }),
    )
    .await;
    let tool = |name: &str, auth_id: &Value| {
        json!({
            "name": name,
            "enabled": false,
            "kind": {
                "type": "http", "method": "GET", "base_url": upstream.uri(),
                "path_template": "/items/{id}",
                "params": [{ "name": "id", "location": "path", "required": true, "schema": { "type": "integer" } }],
                "auth_ref": auth_id,
            }
        })
    };
    let t = post(
        &c,
        format!("{base}/api/projects/{pid}/tools"),
        tool("get_item", &auth["id"]),
    )
    .await;
    let tid = t["id"].as_str().unwrap();

    // MCP view: derived schema.
    let view: Value = c
        .get(format!("{base}/api/projects/{pid}/tools/{tid}/mcp"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(view["name"], "get_item");
    assert_eq!(view["inputSchema"]["required"], json!(["id"]));

    // Success (tool is disabled; the console still runs it).
    let run = post(
        &c,
        format!("{base}/api/projects/{pid}/tools/{tid}/test"),
        json!({ "arguments": { "id": 5 } }),
    )
    .await;
    assert_eq!(run["request"]["method"], "GET");
    assert!(run["request"]["url"]
        .as_str()
        .unwrap()
        .ends_with("/items/5"));
    assert_eq!(run["response"]["status"], 200);
    assert_eq!(run["error"], Value::Null);
    assert_eq!(run["agent_view"]["is_error"], false);
    let text = run["agent_view"]["text"].as_str().unwrap();
    assert!(
        text.contains("[REDACTED]") && !text.contains("key-value-xyz"),
        "{text}"
    );

    // Bad args.
    let run = post(
        &c,
        format!("{base}/api/projects/{pid}/tools/{tid}/test"),
        json!({ "arguments": { "id": "five" } }),
    )
    .await;
    assert_eq!(run["error"]["kind"], "invalid_args");
    assert_eq!(run["error"]["fields"][0]["field"], "id");
    assert_eq!(run["agent_view"]["is_error"], true);

    // Missing secret: config error naming the ref, never a value.
    let t2 = post(
        &c,
        format!("{base}/api/projects/{pid}/tools"),
        tool("get_item2", &missing_auth["id"]),
    )
    .await;
    let run = post(
        &c,
        format!(
            "{base}/api/projects/{pid}/tools/{}/test",
            t2["id"].as_str().unwrap()
        ),
        json!({ "arguments": { "id": 5 } }),
    )
    .await;
    assert_eq!(run["error"]["kind"], "config");
    assert!(run["error"]["message"]
        .as_str()
        .unwrap()
        .contains("NOT_SET"));
    assert_eq!(run["request"], Value::Null);
}
