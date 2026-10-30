//! Stage 17: tools that need approval wait for a person, answered from the
//! web UI or the MCP client (elicitation); first answer wins.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use forge_exec_http::MapSecretResolver;
use forge_store::Store;
use mcp_forge::{AppState, Security};
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, ClientConfig, ContentBlock,
    ElicitRequestParams, ElicitResult, ElicitationAction, Implementation,
};
use rmcp::service::{RequestContext, RoleClient};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::{ClientHandler, ErrorData, ServiceExt};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A client without elicitation: only the web UI can answer.
#[derive(Clone)]
struct Plain;
impl ClientHandler for Plain {}

/// A client that shows approval prompts and answers them.
#[derive(Clone)]
struct Asking {
    answer: Option<ElicitationAction>,
    asked: Arc<AtomicUsize>,
}

impl ClientHandler for Asking {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::new(
            ClientCapabilities::builder().enable_elicitation().build(),
            Implementation::new("asking-client", "1.0"),
        )
    }

    async fn create_elicitation(
        &self,
        request: ElicitRequestParams,
        _context: RequestContext<RoleClient>,
    ) -> Result<ElicitResult, ErrorData> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        let ElicitRequestParams::FormElicitationParams { message, .. } = request else {
            panic!("expected a form prompt");
        };
        assert!(message.contains("delete_user"), "{message}");
        match &self.answer {
            Some(action) => Ok(ElicitResult::new(action.clone())),
            // Never answers: someone else has to.
            None => std::future::pending().await,
        }
    }
}

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
        Mock::given(method("DELETE"))
            .and(path("/users/42"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "deleted": 42 })))
            .mount(&upstream)
            .await;
        let h = Self {
            base,
            http,
            pid: p["id"].as_str().unwrap().to_owned(),
            upstream,
        };
        let tool = h
            .send(
                reqwest::Method::POST,
                &h.project("/tools"),
                json!({
                    "name": "delete_user",
                    "require_approval": true,
                    "kind": {
                        "type": "http",
                        "method": "DELETE",
                        "base_url": h.upstream.uri(),
                        "path_template": "/users/{id}",
                        "params": [{ "name": "id", "location": "path", "required": true, "schema": { "type": "integer" } }]
                    }
                }),
            )
            .await;
        assert_eq!(tool["require_approval"], true);
        h
    }

    fn project(&self, rest: &str) -> String {
        format!("/api/projects/{}{rest}", self.pid)
    }

    async fn send(&self, m: reqwest::Method, url: &str, body: Value) -> Value {
        let r = self
            .http
            .request(m, format!("{}{url}", self.base))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert!(r.status().is_success(), "{}", r.text().await.unwrap());
        r.json().await.unwrap_or(Value::Null)
    }

    async fn get(&self, url: &str) -> Value {
        self.http
            .get(format!("{}{url}", self.base))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    /// Calls `delete_user` over MCP in the background.
    fn call<C: ClientHandler + Clone>(&self, client: C) -> tokio::task::JoinHandle<CallToolResult> {
        let url = format!("{}/mcp/demo", self.base);
        tokio::spawn(async move {
            let running = client
                .serve(StreamableHttpClientTransport::from_uri(url))
                .await
                .unwrap();
            let args = json!({ "id": 42 }).as_object().unwrap().clone();
            let r = running
                .call_tool(CallToolRequestParams::new("delete_user").with_arguments(args))
                .await
                .unwrap();
            running.cancel().await.unwrap();
            r
        })
    }

    /// Waits until exactly one call is pending; returns it.
    async fn pending(&self) -> Value {
        for _ in 0..100 {
            let list = self.get("/api/approvals").await;
            if let Some(p) = list.as_array().and_then(|a| a.first()) {
                return p.clone();
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("no pending approval appeared");
    }

    async fn upstream_hits(&self) -> usize {
        self.upstream.received_requests().await.unwrap().len()
    }

    async fn last_call(&self) -> Value {
        self.get(&self.project("/calls")).await[0].clone()
    }
}

fn text(r: &CallToolResult) -> String {
    match r.content.first() {
        Some(ContentBlock::Text(t)) => t.text.clone(),
        other => panic!("expected text, got {other:?}"),
    }
}

#[tokio::test]
async fn web_ui_approves_and_denies() {
    let h = Harness::start().await;

    let call = h.call(Plain);
    let p = h.pending().await;
    assert_eq!(p["tool_name"], "delete_user");
    assert_eq!(p["project_slug"], "demo");
    assert_eq!(p["arguments"], json!({ "id": 42 }));
    assert_eq!(p["asked_client"], false);
    assert_eq!(h.upstream_hits().await, 0, "nothing runs while waiting");
    let id = p["id"].as_str().unwrap();
    h.send(
        reqwest::Method::POST,
        &format!("/api/approvals/{id}/approve"),
        json!({}),
    )
    .await;
    let r = call.await.unwrap();
    assert_ne!(r.is_error, Some(true), "{}", text(&r));
    assert!(text(&r).contains("deleted"));
    assert_eq!(h.upstream_hits().await, 1);
    let logged = h.last_call().await;
    assert_eq!(logged["status"], "ok");
    assert_eq!(logged["approval"]["decision"], "approved");
    assert_eq!(logged["approval"]["decided_by"], "web UI");

    // Answering twice: the second answer finds nothing pending.
    let r = h
        .http
        .post(format!("{}/api/approvals/{id}/deny", h.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);

    let call = h.call(Plain);
    let id = h.pending().await["id"].as_str().unwrap().to_owned();
    h.send(
        reqwest::Method::POST,
        &format!("/api/approvals/{id}/deny"),
        json!({}),
    )
    .await;
    let r = call.await.unwrap();
    assert_eq!(r.is_error, Some(true));
    assert!(text(&r).contains("denied"), "{}", text(&r));
    assert_eq!(
        h.upstream_hits().await,
        1,
        "denied calls never reach the API"
    );
    let logged = h.last_call().await;
    assert_eq!(logged["status"], "not_approved");
    assert_eq!(logged["approval"]["decision"], "denied");
}

#[tokio::test]
async fn client_prompt_answers() {
    let h = Harness::start().await;

    let asked = Arc::new(AtomicUsize::new(0));
    let r = h
        .call(Asking {
            answer: Some(ElicitationAction::Accept),
            asked: asked.clone(),
        })
        .await
        .unwrap();
    assert_ne!(r.is_error, Some(true), "{}", text(&r));
    assert_eq!(asked.load(Ordering::SeqCst), 1);
    assert_eq!(h.upstream_hits().await, 1);
    let logged = h.last_call().await;
    assert_eq!(logged["approval"]["decision"], "approved");
    assert_eq!(logged["approval"]["decided_by"], "asking-client 1.0");

    let r = h
        .call(Asking {
            answer: Some(ElicitationAction::Decline),
            asked: asked.clone(),
        })
        .await
        .unwrap();
    assert_eq!(r.is_error, Some(true));
    assert!(text(&r).contains("denied"));
    assert_eq!(h.upstream_hits().await, 1);
    assert!(h.get("/api/approvals").await.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn web_ui_can_answer_while_the_client_prompt_is_open() {
    let h = Harness::start().await;
    let asked = Arc::new(AtomicUsize::new(0));
    let call = h.call(Asking {
        answer: None,
        asked: asked.clone(),
    });
    let p = h.pending().await;
    assert_eq!(p["asked_client"], true);
    let id = p["id"].as_str().unwrap();
    h.send(
        reqwest::Method::POST,
        &format!("/api/approvals/{id}/approve"),
        json!({}),
    )
    .await;
    let r = tokio::time::timeout(Duration::from_secs(10), call)
        .await
        .expect("the call finishes once the web UI answers")
        .unwrap();
    assert_ne!(r.is_error, Some(true), "{}", text(&r));
    assert_eq!(asked.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn unanswered_calls_time_out() {
    let h = Harness::start().await;
    let project = h.get(&h.project("")).await;
    let mut settings = project["settings"].clone();
    assert_eq!(settings["approval_timeout_secs"], 300, "default");
    settings["approval_timeout_secs"] = json!(10);
    h.send(reqwest::Method::PUT, &h.project("/settings"), settings)
        .await;

    let r = h.call(Plain).await.unwrap();
    assert_eq!(r.is_error, Some(true));
    assert!(
        text(&r).contains("nobody answered within 10 seconds"),
        "{}",
        text(&r)
    );
    assert_eq!(h.upstream_hits().await, 0);
    let logged = h.last_call().await;
    assert_eq!(logged["approval"]["decision"], "timed_out");
    assert!(logged["approval"]["wait_ms"].as_u64().unwrap() >= 10_000);
    assert!(h.get("/api/approvals").await.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_console_skips_approval() {
    let h = Harness::start().await;
    let tools = h.get(&h.project("/tools")).await;
    let id = tools[0]["id"].as_str().unwrap();
    let run = h
        .send(
            reqwest::Method::POST,
            &h.project(&format!("/tools/{id}/test")),
            json!({ "arguments": { "id": 42 } }),
        )
        .await;
    assert!(run["error"].is_null(), "{run}");
    assert_eq!(h.upstream_hits().await, 1);
    assert!(h.last_call().await["approval"].is_null());
}
