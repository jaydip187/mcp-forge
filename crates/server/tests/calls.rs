//! Stage 16: every tool call lands in the call log, from MCP and the console,
//! with redacted arguments; listing, filters, summary and the off switch.

use std::sync::Arc;

use forge_exec_http::MapSecretResolver;
use forge_store::Store;
use mcp_forge::{AppState, Security};
use rmcp::model::CallToolRequestParams;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::{ClientHandler, ServiceExt};
use serde_json::{json, Map, Value};
use tokio::net::TcpListener;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[derive(Clone)]
struct Client;
impl ClientHandler for Client {}

struct Harness {
    base: String,
    http: reqwest::Client,
    pid: String,
    upstream: MockServer,
}

impl Harness {
    async fn start() -> Self {
        let store = Store::open_in_memory().await.unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(mcp_forge::serve(
            listener,
            AppState::new(store, Arc::new(MapSecretResolver::default())).with_security(Security {
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
        let upstream = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users/1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": 1 })))
            .mount(&upstream)
            .await;
        Mock::given(method("GET"))
            .and(path("/users/2"))
            .respond_with(ResponseTemplate::new(404).set_body_string("no such user"))
            .mount(&upstream)
            .await;
        Self {
            base,
            http,
            pid: p["id"].as_str().unwrap().to_owned(),
            upstream,
        }
    }

    fn url(&self, rest: &str) -> String {
        format!("{}/api/projects/{}{rest}", self.base, self.pid)
    }

    async fn get(&self, rest: &str) -> (reqwest::StatusCode, Value) {
        let r = self.http.get(self.url(rest)).send().await.unwrap();
        let status = r.status();
        (status, r.json().await.unwrap_or(Value::Null))
    }

    async fn send(&self, m: reqwest::Method, rest: &str, body: Value) -> Value {
        let r = self
            .http
            .request(m, self.url(rest))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert!(r.status().is_success(), "{}", r.text().await.unwrap());
        r.json().await.unwrap_or(Value::Null)
    }

    async fn add_tool(&self) -> String {
        let t = self
            .send(
                reqwest::Method::POST,
                "/tools",
                json!({
                    "name": "get_user",
                    "kind": {
                        "type": "http",
                        "method": "GET",
                        "base_url": self.upstream.uri(),
                        "path_template": "/users/{id}",
                        "params": [
                            { "name": "id", "location": "path", "required": true, "schema": { "type": "integer" } },
                            { "name": "api_key", "location": "query" }
                        ]
                    }
                }),
            )
            .await;
        t["id"].as_str().unwrap().to_owned()
    }

    async fn mcp_call(&self, name: &str, args: Value) {
        let transport = StreamableHttpClientTransport::from_uri(format!("{}/mcp/demo", self.base));
        let client = Client.serve(transport).await.unwrap();
        let mut params = CallToolRequestParams::new(name.to_owned());
        if let Value::Object(m) = args {
            params = params.with_arguments(m);
        }
        let _ = client.call_tool(params).await;
        client.cancel().await.unwrap();
    }

    async fn calls(&self, query: &str) -> Vec<Value> {
        let (status, v) = self.get(&format!("/calls{query}")).await;
        assert_eq!(status, 200, "{v}");
        v.as_array().unwrap().clone()
    }
}

#[tokio::test]
async fn mcp_and_console_calls_are_logged() {
    let h = Harness::start().await;
    let tool_id = h.add_tool().await;

    h.mcp_call("get_user", json!({ "id": 1, "api_key": "k-123" }))
        .await;
    h.mcp_call("get_user", json!({ "id": 2 })).await;
    h.mcp_call("no_such_tool", json!({})).await;
    h.send(
        reqwest::Method::POST,
        &format!("/tools/{tool_id}/test"),
        json!({ "arguments": { "id": 1 } }),
    )
    .await;

    let all = h.calls("").await;
    assert_eq!(all.len(), 4, "{all:#?}");
    // Newest first: console, unknown, 404, ok.
    let (console, unknown, failed, ok) = (&all[0], &all[1], &all[2], &all[3]);

    assert_eq!(ok["status"], "ok");
    assert_eq!(ok["origin"], "mcp");
    assert_eq!(ok["tool_id"], tool_id.as_str());
    assert_eq!(ok["upstream_status"], 200);
    assert_eq!(ok["arguments"], json!({ "id": 1, "api_key": "[redacted]" }));
    assert!(ok["client"].as_str().is_some_and(|c| !c.is_empty()), "{ok}");
    assert!(ok["response_chars"].as_u64().unwrap() > 0);
    assert!(ok["error"].is_null());

    assert_eq!(failed["status"], "error");
    assert_eq!(failed["upstream_status"], 404);
    assert!(
        failed["error"].as_str().unwrap().contains("404"),
        "{failed}"
    );

    assert_eq!(unknown["status"], "unknown_tool");
    assert_eq!(unknown["tool_name"], "no_such_tool");
    assert!(unknown["tool_id"].is_null());

    assert_eq!(console["origin"], "console");
    assert!(
        console["client"].is_null(),
        "no signed-in user in this test"
    );

    // Filters and paging.
    assert_eq!(h.calls("?status=error").await.len(), 1);
    assert_eq!(h.calls("?origin=console").await.len(), 1);
    assert_eq!(h.calls("?tool=get_user").await.len(), 3);
    let before = urlencode(ok["ts"].as_str().unwrap());
    assert!(h.calls(&format!("?before={before}")).await.is_empty());
    assert_eq!(h.calls("?limit=1").await.len(), 1);
    let (status, _) = h.get("/calls?status=bogus").await;
    assert_eq!(status, 400);

    // One call.
    let id = failed["id"].as_str().unwrap();
    let (status, one) = h.get(&format!("/calls/{id}")).await;
    assert_eq!((status.as_u16(), &one), (200, failed));

    // Summary.
    let (status, s) = h.get("/calls/summary?window=24h").await;
    assert_eq!(status, 200);
    assert_eq!(
        (s["total"].as_u64(), s["errors"].as_u64()),
        (Some(4), Some(2))
    );
    assert_eq!(s["buckets"].as_array().unwrap().len(), 24);
    assert_eq!(s["tools"][0]["tool_name"], "get_user");
    assert_eq!(s["tools"][0]["calls"], 3);
    let (status, _) = h.get("/calls/summary?window=1y").await;
    assert_eq!(status, 400);
}

#[tokio::test]
async fn zero_days_turns_the_log_off() {
    let h = Harness::start().await;
    h.add_tool().await;
    let (_, project) = h.get("").await;
    let mut settings: Map<String, Value> =
        serde_json::from_value(project["settings"].clone()).unwrap();
    assert_eq!(settings["call_log_days"], 30, "default retention");
    settings.insert("call_log_days".into(), json!(0));
    h.send(
        reqwest::Method::PUT,
        "/settings",
        Value::Object(settings.clone()),
    )
    .await;

    h.mcp_call("get_user", json!({ "id": 1 })).await;
    assert!(h.calls("").await.is_empty());

    settings.insert("call_log_days".into(), json!(400));
    let r = h
        .http
        .put(h.url("/settings"))
        .json(&settings)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);
}

fn urlencode(s: &str) -> String {
    s.replace(':', "%3A").replace('+', "%2B")
}
