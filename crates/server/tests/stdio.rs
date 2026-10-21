//! `mcp-forge stdio`: spawn the real binary and talk MCP over its stdin/stdout.

use std::process::Stdio;
use std::time::Duration;

use forge_core::{ProjectInput, ToolInput};
use forge_store::Store;
use rmcp::model::{CallToolRequestParams, ContentBlock};
use rmcp::service::{NotificationContext, RoleClient};
use rmcp::{ClientHandler, ServiceExt};
use serde_json::{json, Value};
use tokio::process::Command;
use tokio::sync::mpsc;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

#[derive(Clone)]
struct Client {
    changed: mpsc::UnboundedSender<()>,
}

impl ClientHandler for Client {
    async fn on_tool_list_changed(&self, _context: NotificationContext<RoleClient>) {
        let _ = self.changed.send(());
    }
}

fn tool(name: &str, base: &str) -> ToolInput {
    serde_json::from_value(json!({
        "name": name,
        "kind": { "type": "http", "method": "GET", "base_url": base, "path_template": "/ping" }
    }))
    .unwrap()
}

#[tokio::test]
async fn stdio_lists_calls_and_sees_external_edits() {
    let upstream = MockServer::start().await;
    Mock::given(path("/ping"))
        .respond_with(ResponseTemplate::new(200).set_body_string("pong"))
        .mount(&upstream)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(tmp.path()).await.unwrap();
    let project = store
        .create_project(&ProjectInput {
            name: "Demo".into(),
            slug: None,
        })
        .await
        .unwrap();
    store
        .create_tool(project.id, &tool("ping", &upstream.uri()))
        .await
        .unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_mcp-forge"))
        .args(["stdio", "--project", "demo", "--data-dir"])
        .arg(tmp.path())
        .env("RUST_LOG", "warn")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let transport = (child.stdout.take().unwrap(), child.stdin.take().unwrap());
    let (tx, mut changed) = mpsc::unbounded_channel();
    let client = Client { changed: tx }.serve(transport).await.unwrap();

    let tools = client.list_all_tools().await.unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].description.as_deref(), Some("GET /ping"));

    let r = client
        .call_tool(CallToolRequestParams::new("ping"))
        .await
        .unwrap();
    let ContentBlock::Text(t) = &r.content[0] else {
        panic!("expected text")
    };
    assert_eq!(t.text, "pong");

    // An edit from another process (here: this test's own store handle).
    store
        .create_tool(project.id, &tool("ping2", &upstream.uri()))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), changed.recv())
        .await
        .expect("no list_changed within 10s")
        .unwrap();
    let names: Vec<Value> = client
        .list_all_tools()
        .await
        .unwrap()
        .iter()
        .map(|t| json!(t.name))
        .collect();
    assert_eq!(names, [json!("ping"), json!("ping2")]);

    client.cancel().await.unwrap();
    let _ = child.wait().await;
}

#[tokio::test]
async fn stdio_unknown_project_fails_fast() {
    let tmp = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mcp-forge"))
        .args(["stdio", "--project", "nope", "--data-dir"])
        .arg(tmp.path())
        .stdin(Stdio::null())
        .output()
        .await
        .unwrap();
    assert!(!out.status.success());
    assert!(out.stdout.is_empty(), "stdout must stay clean for MCP");
    assert!(String::from_utf8_lossy(&out.stderr).contains("project `nope` not found"));
}
