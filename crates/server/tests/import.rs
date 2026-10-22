//! Stage 4: OpenAPI import over the REST API.

use std::sync::Arc;

use forge_exec_http::MapSecretResolver;
use forge_store::Store;
use mcp_forge::AppState;
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const PETSTORE: &str = include_str!("../../importers/tests/fixtures/petstore3.json");

struct H {
    base: String,
    c: Client,
    pid: String,
}

impl H {
    async fn start() -> Self {
        let store = Store::open_in_memory().await.unwrap();
        let secrets = MapSecretResolver::default().with("env:PET_KEY", "pet-secret");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/api", listener.local_addr().unwrap());
        tokio::spawn(mcp_forge::serve(
            listener,
            AppState::new(store, Arc::new(secrets)),
            std::future::pending(),
        ));
        let c = Client::new();
        let p: Value = c
            .post(format!("{base}/projects"))
            .json(&json!({ "name": "Pets" }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        Self {
            base,
            c,
            pid: p["id"].as_str().unwrap().to_owned(),
        }
    }

    async fn post(&self, p: &str, body: Value) -> (StatusCode, Value) {
        let r = self
            .c
            .post(format!("{}/projects/{}{p}", self.base, self.pid))
            .json(&body)
            .send()
            .await
            .unwrap();
        let st = r.status();
        (st, r.json().await.unwrap_or(Value::Null))
    }

    async fn get(&self, p: &str) -> Value {
        self.c
            .get(format!("{}/projects/{}{p}", self.base, self.pid))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }
}

fn op<'a>(preview: &'a Value, key: &str) -> &'a Value {
    preview["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["key"] == key)
        .unwrap_or_else(|| panic!("no {key}"))
}

#[tokio::test]
async fn preview_from_text_needs_base_url_for_relative_servers() {
    let h = H::start().await;
    let (st, pv) = h
        .post("/import/openapi/preview", json!({ "content": PETSTORE }))
        .await;
    assert_eq!(st, StatusCode::OK, "{pv}");
    assert_eq!(pv["info"]["title"], "Swagger Petstore - OpenAPI 3.0");
    assert_eq!(pv["info"]["servers"], json!(["/api/v3"]));
    assert_eq!(pv["info"]["base_url"], Value::Null);
    assert_eq!(pv["operations"].as_array().unwrap().len(), 19);
    assert_eq!(op(&pv, "GET /pet/{petId}")["supported"], false);
    assert_eq!(
        pv["content"],
        Value::Null,
        "uploaded text isn't echoed back"
    );

    let (_, pv) = h
        .post(
            "/import/openapi/preview",
            json!({ "content": PETSTORE, "base_url": "https://pets.example.com/api/v3" }),
        )
        .await;
    let get = op(&pv, "GET /pet/{petId}");
    assert_eq!(get["supported"], true);
    assert_eq!(get["name"], "getPetById");
    assert_eq!(get["method"], "GET");
    assert_eq!(get["needs_auth"], true);
    assert_eq!(get["tags"], json!(["pet"]));
    assert_eq!(pv["any_auth"], true);
}

#[tokio::test]
async fn preview_from_url_resolves_relative_server_and_echoes_content() {
    let upstream = MockServer::start().await;
    Mock::given(path("/api/v3/openapi.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(PETSTORE))
        .expect(1)
        .mount(&upstream)
        .await;
    let h = H::start().await;
    let url = format!("{}/api/v3/openapi.json", upstream.uri());
    let (st, pv) = h
        .post("/import/openapi/preview", json!({ "url": url }))
        .await;
    assert_eq!(st, StatusCode::OK, "{pv}");
    assert_eq!(pv["info"]["base_url"], format!("{}/api/v3", upstream.uri()));
    assert_eq!(pv["source_url"], url);
    assert_eq!(pv["content"].as_str().unwrap().len(), PETSTORE.len());

    let (st, e) = h
        .post(
            "/import/openapi/preview",
            json!({ "url": format!("{}/missing", upstream.uri()) }),
        )
        .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert!(e["error"]["message"].as_str().unwrap().contains("404"));
    let (st, _) = h
        .post(
            "/import/openapi/preview",
            json!({ "url": "file:///etc/passwd" }),
        )
        .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn commit_creates_tools_that_run() {
    let upstream = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v3/pet/7"))
        .and(header("api_key", "pet-secret"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": 7, "name": "Rex" })))
        .expect(1)
        .mount(&upstream)
        .await;
    let h = H::start().await;
    let (_, auth) = h
        .post(
            "/auth-configs",
            json!({ "name": "pet-key", "kind": { "type": "api_key", "location": "header", "name": "api_key" }, "secret_ref": "env:PET_KEY" }),
        )
        .await;
    // An existing tool that clashes with an imported name.
    let (st, _) = h
        .post(
            "/tools",
            json!({ "name": "getPetById", "kind": { "type": "http", "method": "GET", "base_url": "https://x.io", "path_template": "/" } }),
        )
        .await;
    assert_eq!(st, StatusCode::CREATED);

    let base = format!("{}/api/v3", upstream.uri());
    let (_, pv) = h
        .post(
            "/import/openapi/preview",
            json!({ "content": PETSTORE, "base_url": base }),
        )
        .await;
    assert_eq!(
        op(&pv, "GET /pet/{petId}")["name"],
        "getPetById_2",
        "preview shows the final name"
    );

    let (st, done) = h
        .post(
            "/import/openapi",
            json!({
                "content": PETSTORE,
                "filename": "petstore3.json",
                "base_url": base,
                "auth_ref": auth["id"],
                "operations": [
                    { "key": "GET /pet/{petId}" },
                    { "key": "POST /pet", "name": "create_pet" },
                    { "key": "GET /store/inventory" }
                ]
            }),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{done}");
    let created = done["created"].as_array().unwrap();
    let names: Vec<_> = created
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["getPetById_2", "create_pet", "getInventory"]);
    assert_eq!(created[0]["source"]["type"], "open_api");
    assert_eq!(created[0]["source"]["op_id"], "getPetById");
    assert_eq!(created[0]["source"]["spec_id"], done["spec"]["id"]);
    assert_eq!(created[0]["kind"]["auth_ref"], auth["id"]);

    let specs = h.get("/specs").await;
    assert_eq!(specs[0]["filename"], "petstore3.json");
    assert_eq!(specs[0]["kind"], "openapi3");

    // The imported tool actually works, with the auth applied.
    let tid = created[0]["id"].as_str().unwrap();
    let (st, run) = h
        .post(
            &format!("/tools/{tid}/test"),
            json!({ "arguments": { "petId": 7 } }),
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(run["response"]["status"], 200, "{run}");
    assert_eq!(run["agent_view"]["is_error"], false);
}

#[tokio::test]
async fn commit_rejects_bad_selections_and_content() {
    let h = H::start().await;
    let (st, e) = h
        .post(
            "/import/openapi",
            json!({ "content": PETSTORE, "base_url": "https://p.io", "operations": [{ "key": "GET /nope" }] }),
        )
        .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(e["error"]["fields"][0]["field"], "operations[0]");

    // No base URL → every op is unsupported.
    let (st, e) = h
        .post(
            "/import/openapi",
            json!({ "content": PETSTORE, "operations": [{ "key": "GET /pet/{petId}" }] }),
        )
        .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        e["error"]["message"].as_str().unwrap().contains("base URL"),
        "{e}"
    );

    let (st, e) = h
        .post(
            "/import/openapi/preview",
            json!({ "content": "{ not json" }),
        )
        .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(e["error"]["fields"][0]["field"], "content");
    let (st, e) = h
        .post(
            "/import/openapi/preview",
            json!({ "content": "{\"swagger\":\"2.0\"}" }),
        )
        .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(e["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Swagger 2.0"));

    // Nothing was stored by the failed commits.
    assert_eq!(h.get("/specs").await, json!([]));
}

#[tokio::test]
async fn specs_over_two_megabytes_are_accepted() {
    let h = H::start().await;
    let big = json!({
        "openapi": "3.0.0",
        "info": { "title": "Big", "version": "1", "description": "x".repeat(3 * 1024 * 1024) },
        "servers": [{ "url": "https://big.example.com" }],
        "paths": { "/a": { "get": { "operationId": "a" } } }
    })
    .to_string();
    let (st, pv) = h
        .post("/import/openapi/preview", json!({ "content": big }))
        .await;
    assert_eq!(st, StatusCode::OK, "{pv}");
    assert_eq!(pv["operations"][0]["name"], "a");
}
