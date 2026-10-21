use std::sync::Arc;

use forge_exec_http::MapSecretResolver;
use forge_store::Store;
use mcp_forge::AppState;
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use tokio::net::TcpListener;

struct Api {
    base: String,
    http: Client,
}

impl Api {
    async fn start() -> Self {
        let store = Store::open_in_memory().await.unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/api", listener.local_addr().unwrap());
        tokio::spawn(mcp_forge::serve(
            listener,
            AppState::new(store, Arc::new(MapSecretResolver::default())),
            std::future::pending(),
        ));
        Self {
            base,
            http: Client::new(),
        }
    }

    async fn send(&self, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let url = format!("{}{path}", self.base);
        let mut req = self.http.request(method.parse().unwrap(), url);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().await.unwrap();
        let status = resp.status();
        let text = resp.text().await.unwrap();
        let body = if text.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).unwrap_or_else(|_| panic!("non-JSON body: {text}"))
        };
        (status, body)
    }

    async fn get(&self, path: &str) -> (StatusCode, Value) {
        self.send("GET", path, None).await
    }
    async fn post(&self, path: &str, body: Value) -> (StatusCode, Value) {
        self.send("POST", path, Some(body)).await
    }
    async fn put(&self, path: &str, body: Value) -> (StatusCode, Value) {
        self.send("PUT", path, Some(body)).await
    }
    async fn delete(&self, path: &str) -> (StatusCode, Value) {
        self.send("DELETE", path, None).await
    }

    async fn new_project(&self, name: &str) -> String {
        let (st, p) = self.post("/projects", json!({ "name": name })).await;
        assert_eq!(st, StatusCode::CREATED, "{p}");
        p["id"].as_str().unwrap().to_owned()
    }
}

fn manual_get_tool() -> Value {
    json!({
        "name": "get_user",
        "description": "Fetch one user by id",
        "kind": {
            "type": "http",
            "method": "GET",
            "base_url": "https://api.example.com/v1",
            "path_template": "/users/{id}",
            "params": [
                { "name": "id", "location": "path", "required": true, "description": "User id" },
                { "name": "fields", "location": "query" },
                { "name": "X-Api-Version", "location": "header", "fixed_value": "2024-01-01" }
            ],
            "timeout_ms": 5000
        }
    })
}

/// Stage 1 "done when": create a project + manual HTTP tool via the API and read it back.
#[tokio::test]
async fn create_project_and_manual_http_tool_then_read_back() {
    let api = Api::start().await;

    let (st, project) = api.post("/projects", json!({ "name": "Demo API" })).await;
    assert_eq!(st, StatusCode::CREATED);
    assert_eq!(project["slug"], "demo-api");
    let pid = project["id"].as_str().unwrap();

    let (st, created) = api
        .post(&format!("/projects/{pid}/tools"), manual_get_tool())
        .await;
    assert_eq!(st, StatusCode::CREATED, "{created}");
    let tid = created["id"].as_str().unwrap();
    assert_eq!(created["project_id"], pid);
    assert_eq!(created["enabled"], true);
    assert_eq!(created["source"], json!({ "type": "manual" }));

    let (st, fetched) = api.get(&format!("/projects/{pid}/tools/{tid}")).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(fetched, created);
    let kind = &fetched["kind"];
    assert_eq!(kind["method"], "GET");
    assert_eq!(kind["path_template"], "/users/{id}");
    assert_eq!(kind["timeout_ms"], 5000);
    assert_eq!(kind["params"][0]["schema"], json!({ "type": "string" }));
    assert_eq!(kind["params"][2]["fixed_value"], "2024-01-01");

    let (_, list) = api.get(&format!("/projects/{pid}/tools")).await;
    assert_eq!(list, json!([created]));
    let (_, projects) = api.get("/projects").await;
    assert_eq!(projects, json!([project]));
}

#[tokio::test]
async fn tool_update_and_delete() {
    let api = Api::start().await;
    let pid = api.new_project("Demo").await;
    let (_, t) = api
        .post(&format!("/projects/{pid}/tools"), manual_get_tool())
        .await;
    let tid = t["id"].as_str().unwrap();

    let mut edit = manual_get_tool();
    edit["description"] = json!("Updated");
    edit["enabled"] = json!(false);
    let (st, u) = api.put(&format!("/projects/{pid}/tools/{tid}"), edit).await;
    assert_eq!(st, StatusCode::OK, "{u}");
    assert_eq!(u["description"], "Updated");
    assert_eq!(u["enabled"], false);
    assert_eq!(u["created_at"], t["created_at"]);

    let (st, _) = api.delete(&format!("/projects/{pid}/tools/{tid}")).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (st, e) = api.get(&format!("/projects/{pid}/tools/{tid}")).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    assert_eq!(e["error"]["code"], "not_found");
}

#[tokio::test]
async fn validation_errors_name_the_field() {
    let api = Api::start().await;
    let pid = api.new_project("Demo").await;
    let mut bad = manual_get_tool();
    bad["name"] = json!("get user!");
    bad["kind"]["path_template"] = json!("/users/{user_id}");
    let (st, e) = api.post(&format!("/projects/{pid}/tools"), bad).await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(e["error"]["code"], "validation");
    let fields: Vec<&str> = e["error"]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["field"].as_str().unwrap())
        .collect();
    assert!(fields.contains(&"name"), "{fields:?}");
    assert!(fields.contains(&"kind.params"), "{fields:?}");
    assert!(fields.contains(&"kind.params[0].name"), "{fields:?}");
}

#[tokio::test]
async fn duplicate_names_conflict() {
    let api = Api::start().await;
    let pid = api.new_project("Demo").await;
    let (st, _) = api.post("/projects", json!({ "name": "demo" })).await;
    assert_eq!(st, StatusCode::CONFLICT);

    let path = format!("/projects/{pid}/tools");
    api.post(&path, manual_get_tool()).await;
    let (st, e) = api.post(&path, manual_get_tool()).await;
    assert_eq!(st, StatusCode::CONFLICT);
    assert!(e["error"]["message"].as_str().unwrap().contains("get_user"));
}

#[tokio::test]
async fn malformed_requests_are_400_json() {
    let api = Api::start().await;
    let (st, e) = api.get("/projects/not-a-uuid").await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert_eq!(e["error"]["code"], "bad_request");

    let (st, e) = api.post("/projects", json!({ "nope": 1 })).await;
    assert!(st.is_client_error(), "{st}");
    assert_eq!(e["error"]["code"], "bad_request");

    let (st, e) = api.get("/no-such-route").await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    assert_eq!(e["error"]["code"], "not_found");
}

#[tokio::test]
async fn auth_configs_crud_and_raw_secret_rejected() {
    let api = Api::start().await;
    let pid = api.new_project("Demo").await;
    let path = format!("/projects/{pid}/auth-configs");

    let (st, _) = api
        .post(
            &path,
            json!({ "name": "gh", "kind": { "type": "bearer" }, "secret_ref": "ghp_rawtoken" }),
        )
        .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);

    let (st, cfg) = api
        .post(
            &path,
            json!({ "name": "gh", "kind": { "type": "bearer" }, "secret_ref": "env:GH_TOKEN" }),
        )
        .await;
    assert_eq!(st, StatusCode::CREATED, "{cfg}");
    let aid = cfg["id"].as_str().unwrap();
    let (_, got) = api.get(&format!("{path}/{aid}")).await;
    assert_eq!(got, cfg);

    // Link a tool, then deleting the config conflicts.
    let mut t = manual_get_tool();
    t["kind"]["auth_ref"] = json!(aid);
    let (st, tool) = api.post(&format!("/projects/{pid}/tools"), t).await;
    assert_eq!(st, StatusCode::CREATED, "{tool}");
    let (st, _) = api.delete(&format!("{path}/{aid}")).await;
    assert_eq!(st, StatusCode::CONFLICT);

    let (st, upd) = api
        .put(
            &format!("{path}/{aid}"),
            json!({
                "name": "gh",
                "kind": { "type": "api_key", "location": "header", "name": "X-Key" },
                "secret_ref": "keyring:gh"
            }),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{upd}");
    assert_eq!(upd["kind"]["type"], "api_key");

    let (_, list) = api.get(&path).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn project_update_and_cascade_delete() {
    let api = Api::start().await;
    let pid = api.new_project("Demo").await;
    let (st, p) = api
        .put(&format!("/projects/{pid}"), json!({ "name": "Renamed" }))
        .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(p["slug"], "demo");

    api.post(&format!("/projects/{pid}/tools"), manual_get_tool())
        .await;
    let (st, _) = api.delete(&format!("/projects/{pid}")).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (st, _) = api.get(&format!("/projects/{pid}/tools")).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}
