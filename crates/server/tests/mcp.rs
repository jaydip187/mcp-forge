//! Stage 2 "done when": an MCP client lists and calls a manual GET tool
//! (served over streamable HTTP) against a wiremock upstream.

use std::sync::Arc;
use std::time::Duration;

use forge_exec_http::MapSecretResolver;
use forge_store::Store;
use mcp_forge::{AppState, Security};
use rmcp::model::{CallToolRequestParams, CallToolResult, ContentBlock};
use rmcp::service::{NotificationContext, RoleClient, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::{ClientHandler, ServiceExt};
use serde_json::{json, Map, Value};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Client that forwards `tools/list_changed` notifications to a channel.
#[derive(Clone)]
struct Client {
    changed: mpsc::UnboundedSender<()>,
}

impl ClientHandler for Client {
    async fn on_tool_list_changed(&self, _context: NotificationContext<RoleClient>) {
        let _ = self.changed.send(());
    }
}

struct Harness {
    base: String,
    http: reqwest::Client,
    project_id: String,
    upstream: MockServer,
}

impl Harness {
    async fn start() -> Self {
        let store = Store::open_in_memory().await.unwrap();
        let secrets = MapSecretResolver::default().with("env:UPSTREAM_TOKEN", "s3cret-token");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(mcp_forge::serve(
            listener,
            AppState::new(store, Arc::new(secrets)).with_security(Security {
                require_login: false,
                ..Security::default()
            }),
            std::future::pending(),
        ));
        let http = reqwest::Client::new();
        let p: Value = http
            .post(format!("{base}/api/projects"))
            .json(&json!({ "name": "Demo" }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        Self {
            base,
            http,
            project_id: p["id"].as_str().unwrap().to_owned(),
            upstream: MockServer::start().await,
        }
    }

    async fn api_post(&self, path: &str, body: Value) -> Value {
        let resp = self
            .http
            .post(format!(
                "{}/api/projects/{}{path}",
                self.base, self.project_id
            ))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success(), "{}", resp.text().await.unwrap());
        resp.json().await.unwrap()
    }

    async fn connect(
        &self,
        slug: &str,
    ) -> (
        RunningService<RoleClient, Client>,
        mpsc::UnboundedReceiver<()>,
    ) {
        let (tx, rx) = mpsc::unbounded_channel();
        let transport =
            StreamableHttpClientTransport::from_uri(format!("{}/mcp/{slug}", self.base));
        let client = Client { changed: tx }.serve(transport).await.unwrap();
        (client, rx)
    }
}

fn get_user_tool(base_url: &str, auth_ref: Option<&str>) -> Value {
    json!({
        "name": "get_user",
        "description": "Fetch one user by id",
        "kind": {
            "type": "http",
            "method": "GET",
            "base_url": base_url,
            "path_template": "/users/{id}",
            "params": [
                { "name": "id", "location": "path", "required": true, "schema": { "type": "integer" } },
                { "name": "fields", "location": "query" },
                { "name": "Accept", "location": "header", "fixed_value": "application/json" }
            ],
            "auth_ref": auth_ref,
        }
    })
}

fn text(r: &CallToolResult) -> String {
    r.content
        .iter()
        .filter_map(|c| match c {
            ContentBlock::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn args(v: Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}

#[tokio::test]
async fn mcp_client_lists_and_calls_manual_get_tool() {
    let h = Harness::start().await;
    Mock::given(method("GET"))
        .and(path("/users/42"))
        .and(query_param("fields", "name"))
        .and(header("Accept", "application/json"))
        .and(header("Authorization", "Bearer s3cret-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": 42, "name": "Ada" })))
        .expect(1)
        .mount(&h.upstream)
        .await;
    let auth = h
        .api_post(
            "/auth-configs",
            json!({ "name": "token", "kind": { "type": "bearer" }, "secret_ref": "env:UPSTREAM_TOKEN" }),
        )
        .await;
    h.api_post(
        "/tools",
        get_user_tool(&h.upstream.uri(), auth["id"].as_str()),
    )
    .await;

    let (client, _rx) = h.connect("demo").await;

    let tools = client.list_all_tools().await.unwrap();
    assert_eq!(tools.len(), 1);
    let t = &tools[0];
    assert_eq!(t.name, "get_user");
    assert_eq!(t.description.as_deref(), Some("Fetch one user by id"));
    assert_eq!(
        Value::Object((*t.input_schema).clone()),
        json!({
            "type": "object",
            "properties": { "id": { "type": "integer" }, "fields": { "type": "string" } },
            "required": ["id"],
            "additionalProperties": false
        })
    );
    assert_eq!(
        t.annotations.as_ref().and_then(|a| a.read_only_hint),
        Some(true)
    );

    let r = client
        .call_tool(
            CallToolRequestParams::new("get_user")
                .with_arguments(args(json!({ "id": 42, "fields": "name" }))),
        )
        .await
        .unwrap();
    assert_ne!(r.is_error, Some(true), "{}", text(&r));
    assert_eq!(
        serde_json::from_str::<Value>(&text(&r)).unwrap(),
        json!({ "id": 42, "name": "Ada" })
    );

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn llm_friendly_errors() {
    let h = Harness::start().await;
    Mock::given(path("/users/7"))
        .respond_with(
            ResponseTemplate::new(404).set_body_json(json!({ "message": "user 7 not found" })),
        )
        .mount(&h.upstream)
        .await;
    h.api_post("/tools", get_user_tool(&h.upstream.uri(), None))
        .await;
    let (client, _rx) = h.connect("demo").await;

    // Upstream error: status + body, flagged as a tool error.
    let r = client
        .call_tool(CallToolRequestParams::new("get_user").with_arguments(args(json!({ "id": 7 }))))
        .await
        .unwrap();
    assert_eq!(r.is_error, Some(true));
    let t = text(&r);
    assert!(
        t.contains("HTTP 404 Not Found") && t.contains("user 7 not found"),
        "{t}"
    );

    // Bad args: the error names the argument.
    let r = client
        .call_tool(
            CallToolRequestParams::new("get_user").with_arguments(args(json!({ "id": "seven" }))),
        )
        .await
        .unwrap();
    assert_eq!(r.is_error, Some(true));
    let t = text(&r);
    assert!(t.contains("`id`") && t.contains("expected integer"), "{t}");

    // Unknown tool: protocol error.
    let err = client
        .call_tool(CallToolRequestParams::new("nope"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("unknown tool"), "{err}");

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn tool_edits_emit_list_changed() {
    let h = Harness::start().await;
    let (client, mut changed) = h.connect("demo").await;
    assert!(client.list_all_tools().await.unwrap().is_empty());

    h.api_post("/tools", get_user_tool(&h.upstream.uri(), None))
        .await;
    tokio::time::timeout(Duration::from_secs(5), changed.recv())
        .await
        .expect("no tools/list_changed notification within 5s")
        .unwrap();
    assert_eq!(client.list_all_tools().await.unwrap().len(), 1);

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn unknown_project_is_404() {
    let h = Harness::start().await;
    let resp = h
        .http
        .post(format!("{}/mcp/missing", h.base))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}
