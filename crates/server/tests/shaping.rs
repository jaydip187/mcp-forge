//! Stage 7: response shaping (test console + MCP), project filters, tool search.

use std::sync::Arc;

use forge_exec_http::MapSecretResolver;
use forge_store::Store;
use mcp_forge::{AppState, Security};
use reqwest::{Client, StatusCode};
use rmcp::model::{CallToolRequestParams, CallToolResult, ContentBlock};
use rmcp::service::RunningService;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{json, Map, Value};
use tokio::net::TcpListener;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

struct H {
    root: String,
    base: String,
    c: Client,
    pid: String,
    upstream: MockServer,
}

impl H {
    async fn start() -> Self {
        let store = Store::open_in_memory().await.unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let root = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(mcp_forge::serve(
            listener,
            AppState::new(store, Arc::new(MapSecretResolver::default())).with_security(Security {
                require_login: false,
                ..Security::default()
            }),
            std::future::pending(),
        ));
        let c = Client::new();
        let p: Value = c
            .post(format!("{root}/api/projects"))
            .json(&json!({ "name": "Shop" }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let upstream = MockServer::start().await;
        Mock::given(path("/users"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                { "id": 1, "name": "Ada", "email": null, "bio": "x".repeat(900) },
                { "id": 2, "name": "Linus", "email": "l@x.io", "bio": null }
            ])))
            .mount(&upstream)
            .await;
        Mock::given(path("/orders"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "ok": true })))
            .mount(&upstream)
            .await;
        Self {
            base: format!("{root}/api/projects/{}", p["id"].as_str().unwrap()),
            root,
            c,
            pid: p["id"].as_str().unwrap().to_owned(),
            upstream,
        }
    }

    async fn send(&self, method: &str, p: &str, body: Value) -> (StatusCode, Value) {
        let r = self
            .c
            .request(method.parse().unwrap(), format!("{}{p}", self.base))
            .json(&body)
            .send()
            .await
            .unwrap();
        let st = r.status();
        (st, r.json().await.unwrap_or(Value::Null))
    }

    async fn tool(
        &self,
        name: &str,
        method: &str,
        p: &str,
        tags: &[&str],
        shape: Option<&str>,
    ) -> String {
        let (st, t) = self
            .send(
                "POST",
                "/tools",
                json!({
                    "name": name, "tags": tags, "response_shape": shape,
                    "kind": { "type": "http", "method": method, "base_url": self.upstream.uri(), "path_template": p }
                }),
            )
            .await;
        assert_eq!(st, StatusCode::CREATED, "{t}");
        t["id"].as_str().unwrap().to_owned()
    }

    async fn settings(&self, s: Value) {
        let (st, p) = self.send("PUT", "/settings", s).await;
        assert_eq!(st, StatusCode::OK, "{p}");
    }

    async fn mcp(&self) -> RunningService<RoleClient, ()> {
        let slug = "shop";
        let t = StreamableHttpClientTransport::from_uri(format!("{}/mcp/{slug}", self.root));
        ().serve(t).await.unwrap()
    }
}

async fn names(c: &RunningService<RoleClient, ()>) -> Vec<String> {
    let mut n: Vec<String> = c
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    n.sort();
    n
}

fn text(r: &CallToolResult) -> String {
    r.content
        .iter()
        .filter_map(|c| match c {
            ContentBlock::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect()
}

fn call(name: &str, args: Value) -> CallToolRequestParams {
    let m: Map<String, Value> = args.as_object().cloned().unwrap_or_default();
    CallToolRequestParams::new(name.to_owned()).with_arguments(m)
}

/// Stage 7 "done when": shaped output in the test console (and the same over MCP).
#[tokio::test]
async fn test_console_shows_shaped_output() {
    let h = H::start().await;
    let id = h
        .tool("list_users", "GET", "/users", &[], Some("map({id, name})"))
        .await;

    let (st, run) = h
        .send("POST", &format!("/tools/{id}/test"), json!({}))
        .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(
        run["shaped"],
        r#"[{"id":1,"name":"Ada"},{"id":2,"name":"Linus"}]"#
    );
    assert_eq!(run["agent_view"]["text"], run["shaped"]);
    assert!(
        run["response"]["body"].as_str().unwrap().contains("bio"),
        "raw response stays raw"
    );

    let client = h.mcp().await;
    let r = client
        .call_tool(call("list_users", json!({})))
        .await
        .unwrap();
    assert_eq!(
        text(&r),
        r#"[{"id":1,"name":"Ada"},{"id":2,"name":"Linus"}]"#
    );
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn null_stripping_and_truncation_from_settings() {
    let h = H::start().await;
    let id = h.tool("list_users", "GET", "/users", &[], None).await;

    // No shaping configured → console reports nothing shaped.
    let (_, run) = h
        .send("POST", &format!("/tools/{id}/test"), json!({}))
        .await;
    assert_eq!(run["shaped"], Value::Null);

    h.settings(json!({ "strip_nulls": true })).await;
    let (_, run) = h
        .send("POST", &format!("/tools/{id}/test"), json!({}))
        .await;
    let shaped = run["shaped"].as_str().unwrap();
    assert!(!shaped.contains("null"), "{shaped}");

    h.settings(json!({ "max_response_chars": 500 })).await;
    let (_, run) = h
        .send("POST", &format!("/tools/{id}/test"), json!({}))
        .await;
    let shaped = run["shaped"].as_str().unwrap();
    assert!(
        shaped.contains("[Response truncated: showing the first 500 of"),
        "{shaped}"
    );

    let (st, e) = h
        .send("PUT", "/settings", json!({ "max_response_chars": 5 }))
        .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(e["error"]["fields"][0]["field"], "max_response_chars");
}

#[tokio::test]
async fn invalid_shape_rejected_on_save() {
    let h = H::start().await;
    let (st, e) = h
        .send(
            "POST",
            "/tools",
            json!({
                "name": "t", "response_shape": "map(.id",
                "kind": { "type": "http", "method": "GET", "base_url": "https://x.io", "path_template": "/" }
            }),
        )
        .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    assert_eq!(e["error"]["fields"][0]["field"], "response_shape");
}

#[tokio::test]
async fn project_filters_hide_tools_from_agents() {
    let h = H::start().await;
    h.tool("list_users", "GET", "/users", &["users"], None)
        .await;
    h.tool("create_order", "POST", "/orders", &["orders"], None)
        .await;
    h.tool(
        "admin_users",
        "GET",
        "/admin/users",
        &["users", "admin"],
        None,
    )
    .await;
    let client = h.mcp().await;
    assert_eq!(names(&client).await.len(), 3);

    h.settings(json!({ "read_only": true })).await;
    assert_eq!(names(&client).await, ["admin_users", "list_users"]);
    let err = client
        .call_tool(call("create_order", json!({})))
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("unknown tool"),
        "hidden tools can't be called: {err}"
    );

    h.settings(json!({ "exclude_tags": ["admin"] })).await;
    assert_eq!(names(&client).await, ["create_order", "list_users"]);

    h.settings(json!({ "include_paths": ["/users"], "exclude_paths": [] }))
        .await;
    // Substring match: "/users" matches both user paths, admin is excluded by nothing now.
    assert_eq!(names(&client).await, ["admin_users", "list_users"]);
    h.settings(json!({ "include_paths": ["/users"], "exclude_paths": ["/admin/**"] }))
        .await;
    assert_eq!(names(&client).await, ["list_users"]);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn tool_search_mode() {
    let h = H::start().await;
    h.tool(
        "list_users",
        "GET",
        "/users",
        &["users"],
        Some("map(.name)"),
    )
    .await;
    h.tool("create_order", "POST", "/orders", &["orders"], None)
        .await;
    h.tool("delete_everything", "DELETE", "/danger", &["admin"], None)
        .await;
    h.settings(json!({ "tool_search": true, "exclude_tags": ["admin"] }))
        .await;
    let client = h.mcp().await;

    let mut listed: Vec<String> = client
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    listed.sort();
    assert_eq!(listed, ["call_tool", "search_tools"]);

    let r = client
        .call_tool(call("search_tools", json!({ "query": "users" })))
        .await
        .unwrap();
    let found: Value = serde_json::from_str(&text(&r)).unwrap();
    assert_eq!(found[0]["name"], "list_users");
    assert!(found[0]["input_schema"].is_object());
    let r = client
        .call_tool(call("search_tools", json!({ "query": "everything" })))
        .await
        .unwrap();
    assert!(
        text(&r).starts_with("No tools match"),
        "hidden tools aren't searchable: {}",
        text(&r)
    );

    let r = client
        .call_tool(call(
            "call_tool",
            json!({ "name": "list_users", "arguments": {} }),
        ))
        .await
        .unwrap();
    assert_eq!(text(&r), r#"["Ada","Linus"]"#, "proxied call is shaped too");
    let err = client
        .call_tool(call("call_tool", json!({ "name": "delete_everything" })))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("unknown tool"));
    client.cancel().await.unwrap();
    let _ = &h.pid;
}
