//! Stage 20: per-project access keys for /mcp: required when the project
//! asks, tool lists enforced on list and call, expiry, other projects'
//! keys refused, admin token still accepted, key name in the call log.

use std::sync::Arc;

use forge_exec_http::MapSecretResolver;
use forge_store::Store;
use mcp_forge::{AppState, Security};
use rmcp::model::CallToolRequestParams;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::{ClientHandler, ServiceExt};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

#[derive(Clone)]
struct Client;
impl ClientHandler for Client {}

struct Harness {
    base: String,
    http: reqwest::Client,
    store: Store,
    _upstream: MockServer,
}

impl Harness {
    async fn start(security: Security) -> Self {
        let store = Store::open_in_memory().await.unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(mcp_forge::serve(
            listener,
            AppState::new(store.clone(), Arc::new(MapSecretResolver::default()))
                .with_security(security),
            std::future::pending(),
        ));
        let upstream = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "ok": true })))
            .mount(&upstream)
            .await;
        let h = Self {
            base,
            http: reqwest::Client::new(),
            store,
            _upstream: upstream,
        };
        for project in ["demo", "other"] {
            let p = h
                .api(
                    reqwest::Method::POST,
                    "/api/projects",
                    json!({ "name": project }),
                )
                .await;
            let pid = p["id"].as_str().unwrap().to_owned();
            for tool in ["read_a", "read_b"] {
                h.api(
                    reqwest::Method::POST,
                    &format!("/api/projects/{pid}/tools"),
                    json!({ "name": tool, "kind": {
                        "type": "http", "method": "GET",
                        "base_url": h._upstream.uri(), "path_template": format!("/{tool}"), "params": []
                    }}),
                )
                .await;
            }
        }
        h
    }

    fn admin() -> Security {
        Security {
            require_login: false,
            ..Security::default()
        }
    }

    async fn api(&self, m: reqwest::Method, path: &str, body: Value) -> Value {
        let r = self
            .http
            .request(m, format!("{}{path}", self.base))
            .bearer_auth("admin-token-123456789")
            .json(&body)
            .send()
            .await
            .unwrap();
        assert!(r.status().is_success(), "{}", r.text().await.unwrap());
        r.json().await.unwrap_or(Value::Null)
    }

    async fn project(&self, slug: &str) -> Value {
        let all = self
            .api(reqwest::Method::GET, "/api/projects", Value::Null)
            .await;
        all.as_array()
            .unwrap()
            .iter()
            .find(|p| p["slug"] == slug)
            .unwrap()
            .clone()
    }

    async fn key(&self, slug: &str, body: Value) -> String {
        let pid = self.project(slug).await["id"].as_str().unwrap().to_owned();
        let r = self
            .api(
                reqwest::Method::POST,
                &format!("/api/projects/{pid}/api-keys"),
                body,
            )
            .await;
        assert!(
            r["key"].get("key_hash").is_none(),
            "hash never leaves the server"
        );
        r["secret"].as_str().unwrap().to_owned()
    }

    async fn require_key(&self, slug: &str) {
        let p = self.project(slug).await;
        let mut settings = p["settings"].clone();
        settings["require_key"] = json!(true);
        self.api(
            reqwest::Method::PUT,
            &format!("/api/projects/{}/settings", p["id"].as_str().unwrap()),
            settings,
        )
        .await;
    }

    /// Raw `initialize` POST: the status says whether the guard let it in.
    async fn initialize_status(&self, slug: &str, bearer: Option<&str>) -> u16 {
        let mut req = self
            .http
            .post(format!("{}/mcp/{slug}", self.base))
            .header("accept", "application/json, text/event-stream")
            .json(&json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": { "protocolVersion": "2025-06-18", "capabilities": {},
                            "clientInfo": { "name": "t", "version": "1" } }
            }));
        if let Some(b) = bearer {
            req = req.bearer_auth(b);
        }
        req.send().await.unwrap().status().as_u16()
    }

    async fn connect(
        &self,
        slug: &str,
        key: &str,
    ) -> rmcp::service::RunningService<rmcp::RoleClient, Client> {
        let config =
            StreamableHttpClientTransportConfig::with_uri(format!("{}/mcp/{slug}", self.base))
                .auth_header(key);
        Client
            .serve(StreamableHttpClientTransport::from_config(config))
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn keys_gate_and_scope_mcp() {
    // Loopback, no admin token: /mcp is open until a project asks for keys.
    let h = Harness::start(Harness::admin()).await;
    assert_eq!(h.initialize_status("demo", None).await, 200);
    h.require_key("demo").await;
    assert_eq!(h.initialize_status("demo", None).await, 401);
    assert_eq!(
        h.initialize_status("demo", Some("mcpf_not-a-real-key"))
            .await,
        401
    );
    assert_eq!(h.initialize_status("other", None).await, 200, "per project");

    let all = h.key("demo", json!({ "name": "laptop" })).await;
    let only_a = h
        .key("demo", json!({ "name": "ci", "tools": ["read_a"] }))
        .await;
    let other = h.key("other", json!({ "name": "elsewhere" })).await;
    assert!(all.starts_with("mcpf_") && all.len() > 40);
    assert_eq!(h.initialize_status("demo", Some(&all)).await, 200);
    assert_eq!(
        h.initialize_status("demo", Some(&other)).await,
        401,
        "other project's key"
    );

    // Tool lists: the scoped key sees and calls only read_a.
    let c = h.connect("demo", &all).await;
    let names: Vec<String> = c
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    assert_eq!(names, ["read_a", "read_b"]);
    c.cancel().await.unwrap();

    let c = h.connect("demo", &only_a).await;
    let names: Vec<String> = c
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    assert_eq!(names, ["read_a"]);
    let ok = c
        .call_tool(CallToolRequestParams::new("read_a"))
        .await
        .unwrap();
    assert_ne!(ok.is_error, Some(true));
    let refused = c.call_tool(CallToolRequestParams::new("read_b")).await;
    assert!(
        refused.unwrap_err().to_string().contains("unknown tool"),
        "outside the key's list"
    );
    c.cancel().await.unwrap();

    // The call log names the key.
    let pid = h.project("demo").await["id"].as_str().unwrap().to_owned();
    let calls = h
        .api(
            reqwest::Method::GET,
            &format!("/api/projects/{pid}/calls"),
            Value::Null,
        )
        .await;
    assert_eq!(calls[0]["tool_name"], "read_b");
    assert_eq!(calls[0]["status"], "unknown_tool");
    assert_eq!(calls[0]["api_key"], "ci");
    assert_eq!(calls[1]["api_key"], "ci");

    // Listing shows when keys were used; deleting a key locks it out.
    let keys = h
        .api(
            reqwest::Method::GET,
            &format!("/api/projects/{pid}/api-keys"),
            Value::Null,
        )
        .await;
    let ci = keys
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["name"] == "ci")
        .unwrap();
    assert!(ci["last_used_at"].is_string());
    assert_eq!(ci["tools"], json!(["read_a"]));
    assert!(ci["prefix"].as_str().unwrap().starts_with("mcpf_"));
    h.api(
        reqwest::Method::DELETE,
        &format!(
            "/api/projects/{pid}/api-keys/{}",
            ci["id"].as_str().unwrap()
        ),
        Value::Null,
    )
    .await;
    assert_eq!(h.initialize_status("demo", Some(&only_a)).await, 401);
}

#[tokio::test]
async fn expired_keys_and_validation() {
    let h = Harness::start(Harness::admin()).await;
    let key = h.key("demo", json!({ "name": "short" })).await;
    // Expire it directly in the store.
    sqlx::query("UPDATE api_keys SET expires_at = '2020-01-01T00:00:00Z'")
        .execute(h.store.pool())
        .await
        .unwrap();
    assert_eq!(h.initialize_status("demo", Some(&key)).await, 401);

    let pid = h.project("demo").await["id"].as_str().unwrap().to_owned();
    for (body, field) in [
        (json!({ "name": "bad name" }), "name"),
        (json!({ "name": "k", "tools": [] }), "tools"),
        (
            json!({ "name": "k", "expires_at": "2020-01-01T00:00:00Z" }),
            "expires_at",
        ),
    ] {
        let r = h
            .http
            .post(format!("{}/api/projects/{pid}/api-keys", h.base))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 422, "{body}");
        let v: Value = r.json().await.unwrap();
        assert!(v.to_string().contains(field), "{v}");
    }
    let r = h
        .http
        .post(format!("{}/api/projects/{pid}/api-keys", h.base))
        .json(&json!({ "name": "short" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 409, "names are unique per project");
}

#[tokio::test]
async fn admin_token_and_keys_off_loopback() {
    // Non-loopback setup: the admin token is required, but a project key
    // also gets in (that's what keys are for).
    let h = Harness::start(Security {
        admin_token: Some("admin-token-123456789".into()),
        loopback_hosts_only: false,
        require_login: false,
    })
    .await;
    assert_eq!(h.initialize_status("demo", None).await, 401);
    assert_eq!(
        h.initialize_status("demo", Some("admin-token-123456789"))
            .await,
        200
    );
    let key = h.key("demo", json!({ "name": "remote" })).await;
    assert_eq!(h.initialize_status("demo", Some(&key)).await, 200);
    assert_eq!(h.initialize_status("other", Some(&key)).await, 401);
}
