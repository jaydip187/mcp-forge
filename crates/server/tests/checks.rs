//! `POST …/check`: the validation of create, without saving, listing every
//! problem at once. The UI calls it from each step's Next button.

use std::sync::Arc;

use forge_exec_http::MapSecretResolver;
use forge_store::Store;
use mcp_forge::{AppState, Security};
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use tokio::net::TcpListener;

struct Api {
    root: String,
    c: Client,
}

impl Api {
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
        Self {
            root,
            c: Client::new(),
        }
    }

    async fn post(&self, path: &str, body: Value) -> (StatusCode, Value) {
        let r = self
            .c
            .post(format!("{}/api{path}", self.root))
            .json(&body)
            .send()
            .await
            .unwrap();
        let st = r.status();
        (st, r.json::<Value>().await.unwrap_or(Value::Null))
    }

    /// The fields a check complains about, or none when it passes.
    async fn check(&self, path: &str, body: Value) -> Vec<String> {
        let (st, e) = self.post(path, body).await;
        if st == StatusCode::NO_CONTENT {
            return vec![];
        }
        assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
        e["error"]["fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["field"].as_str().unwrap().to_owned())
            .collect()
    }

    async fn count(&self, path: &str) -> usize {
        let r = self
            .c
            .get(format!("{}/api{path}", self.root))
            .send()
            .await
            .unwrap();
        r.json::<Value>().await.unwrap().as_array().unwrap().len()
    }
}

fn tool(name: &str, base_url: &str, shape: &str) -> Value {
    json!({
        "name": name,
        "description": "Fetch one user",
        "response_shape": if shape.is_empty() { Value::Null } else { json!(shape) },
        "kind": { "type": "http", "method": "GET", "base_url": base_url, "path_template": "/users/{id}",
                  "params": [{ "name": "id", "location": "path", "required": true }] }
    })
}

#[tokio::test]
async fn checks_report_every_problem_without_saving() {
    let api = Api::start().await;

    let (st, p) = api.post("/projects", json!({ "name": "Shop" })).await;
    assert_eq!(st, StatusCode::CREATED, "{p}");
    let base = format!("/projects/{}", p["id"].as_str().unwrap());

    // Tools: problems from different checks all come back together.
    let bad = api
        .check(
            &format!("{base}/tools/check"),
            tool("bad name", "not a url", ".[ broken"),
        )
        .await;
    for f in ["name", "kind.base_url", "response_shape"] {
        assert!(bad.contains(&f.to_owned()), "{f} missing from {bad:?}");
    }
    let good = tool("get_user", "https://api.example.com", "");
    assert_eq!(
        api.check(&format!("{base}/tools/check"), good.clone())
            .await,
        Vec::<String>::new()
    );
    assert_eq!(
        api.count(&format!("{base}/tools")).await,
        0,
        "a check saves nothing"
    );
    let (st, t) = api.post(&format!("{base}/tools"), good.clone()).await;
    assert_eq!(st, StatusCode::CREATED, "{t}");
    assert_eq!(
        api.check(&format!("{base}/tools/check"), good).await,
        ["name"],
        "taken"
    );

    // Auth configs: a bad secret reference is a field error, and the rest is still checked.
    let fields = api
        .check(
            &format!("{base}/auth-configs/check"),
            json!({ "name": "", "kind": { "type": "bearer" }, "secret_ref": "ghp_rawtoken" }),
        )
        .await;
    assert!(
        fields.contains(&"secret_ref".to_owned()) && fields.contains(&"name".to_owned()),
        "{fields:?}"
    );
    let (st, e) = api
        .post(
            &format!("{base}/auth-configs/check"),
            json!({ "name": "gh", "kind": { "type": "bearer" }, "secret_ref": "ghp_rawtoken" }),
        )
        .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        !e.to_string().contains("ghp_rawtoken"),
        "the value is never echoed: {e}"
    );
    assert!(api
        .check(
            &format!("{base}/auth-configs/check"),
            json!({ "name": "gh", "kind": { "type": "bearer" }, "secret_ref": "env:GH_TOKEN" }),
        )
        .await
        .is_empty());

    // Editing (`?id=`) may keep its own name; another item can't take it.
    let gh = json!({ "name": "gh", "kind": { "type": "bearer" }, "secret_ref": "env:GH_TOKEN" });
    let (st, cfg) = api.post(&format!("{base}/auth-configs"), gh.clone()).await;
    assert_eq!(st, StatusCode::CREATED, "{cfg}");
    assert_eq!(
        api.check(&format!("{base}/auth-configs/check"), gh.clone())
            .await,
        ["name"]
    );
    let own = format!(
        "{base}/auth-configs/check?id={}",
        cfg["id"].as_str().unwrap()
    );
    assert!(api.check(&own, gh).await.is_empty());

    // Connections: reference, full access confirmation and name together.
    let fields = api
        .check(
            &format!("{base}/db-connections/check"),
            json!({ "name": "", "driver": "sqlite", "dsn_ref": "", "access_mode": "full_access" }),
        )
        .await;
    for f in ["name", "dsn_ref", "access_mode"] {
        assert!(
            fields.contains(&f.to_owned()),
            "{f} missing from {fields:?}"
        );
    }
    assert!(api
        .check(
            &format!("{base}/db-connections/check"),
            json!({ "name": "shop", "driver": "sqlite", "dsn_ref": "env:SHOP_DB" }),
        )
        .await
        .is_empty());

    // Resources and prompts.
    assert_eq!(
        api.check(
            &format!("{base}/resources/check"),
            json!({ "name": "x", "mime_type": "application/json", "content": "{nope" }),
        )
        .await,
        ["content"]
    );
    let prompt = json!({ "name": "triage", "template": "Hi {{who}}", "arguments": [] });
    assert!(!api
        .check(&format!("{base}/prompts/check"), prompt)
        .await
        .is_empty());

    // Access keys.
    assert_eq!(
        api.check(&format!("{base}/api-keys/check"), json!({ "name": "" }))
            .await,
        ["name"]
    );
    assert!(api
        .check(
            &format!("{base}/api-keys/check"),
            json!({ "name": "laptop" })
        )
        .await
        .is_empty());
}
