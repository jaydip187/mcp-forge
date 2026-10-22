//! Stage 6 "done when": edit description → re-import changed spec →
//! description preserved, new op shown in the diff.

use std::sync::Arc;

use forge_exec_http::MapSecretResolver;
use forge_store::Store;
use mcp_forge::{AppState, Security};
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use tokio::net::TcpListener;

fn spec_v1() -> Value {
    json!({
        "openapi": "3.0.3",
        "info": { "title": "Users", "version": "1.0" },
        "servers": [{ "url": "https://users.example.com" }],
        "paths": {
            "/users": {
                "get": { "operationId": "listUsers", "summary": "List users" }
            },
            "/users/{id}": {
                "get": {
                    "operationId": "getUser", "summary": "Get a user",
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }]
                },
                "delete": {
                    "operationId": "deleteUser", "summary": "Delete a user",
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }]
                }
            }
        }
    })
}

fn spec_v2() -> Value {
    let mut v = spec_v1();
    v["info"]["version"] = json!("2.0");
    // getUser: new summary in the spec and a new query param.
    v["paths"]["/users/{id}"]["get"]["summary"] = json!("Fetch one user (v2)");
    v["paths"]["/users/{id}"]["get"]["parameters"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "name": "expand", "in": "query", "schema": { "type": "boolean" } }));
    // deleteUser removed; createUser added.
    v["paths"]["/users/{id}"]
        .as_object_mut()
        .unwrap()
        .remove("delete");
    v["paths"]["/users"]["post"] = json!({
        "operationId": "createUser", "summary": "Create a user",
        "requestBody": { "required": true, "content": { "application/json": { "schema": {
            "type": "object", "required": ["email"], "properties": { "email": { "type": "string" } }
        }}}}
    });
    v
}

struct H {
    base: String,
    c: Client,
    pid: String,
}

impl H {
    async fn start() -> Self {
        let store = Store::open_in_memory().await.unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/api", listener.local_addr().unwrap());
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
            .post(format!("{base}/projects"))
            .json(&json!({ "name": "Users" }))
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

    async fn send(&self, method: &str, p: &str, body: Value) -> (StatusCode, Value) {
        let r = self
            .c
            .request(
                method.parse().unwrap(),
                format!("{}/projects/{}{p}", self.base, self.pid),
            )
            .json(&body)
            .send()
            .await
            .unwrap();
        let st = r.status();
        (st, r.json().await.unwrap_or(Value::Null))
    }

    async fn tool(&self, name: &str) -> Value {
        let (_, tools) = self.send("GET", "/tools", Value::Null).await;
        tools
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == name)
            .cloned()
            .unwrap_or(Value::Null)
    }
}

#[tokio::test]
async fn reimport_keeps_edits_and_reports_diff() {
    let h = H::start().await;
    let (st, done) = h
        .send(
            "POST",
            "/import/openapi",
            json!({
                "content": spec_v1().to_string(),
                "operations": [{ "key": "GET /users" }, { "key": "GET /users/{id}" }, { "key": "DELETE /users/{id}" }]
            }),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{done}");
    let spec_id = done["spec"]["id"].as_str().unwrap().to_owned();

    // The user edits getUser's description.
    let mut get_user = h.tool("getUser").await;
    let tid = get_user["id"].as_str().unwrap().to_owned();
    get_user["description"] = json!("My hand-written description.");
    let (st, edited) = h.send("PUT", &format!("/tools/{tid}"), get_user).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(edited["overrides"]["fields"], json!(["description"]));

    // Preview the re-import of v2.
    let reimport = format!("/specs/{spec_id}/reimport");
    let (st, pv) = h
        .send(
            "POST",
            &format!("{reimport}/preview"),
            json!({ "content": spec_v2().to_string() }),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{pv}");
    assert_eq!(pv["info"]["api_version"], "2.0");
    assert_eq!(pv["unchanged"], 1, "listUsers didn't change: {pv}");
    let changed = pv["changed"].as_array().unwrap();
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0]["name"], "getUser");
    assert_eq!(
        changed[0]["fields"],
        json!(["kind.params"]),
        "description is kept, so it isn't a change"
    );
    assert_eq!(changed[0]["kept"], json!(["description"]));
    let added = pv["added"].as_array().unwrap();
    assert_eq!(added.len(), 1);
    assert_eq!(added[0]["key"], "POST /users");
    assert_eq!(added[0]["name"], "createUser");
    assert_eq!(pv["removed"][0]["name"], "deleteUser");
    let delete_id = pv["removed"][0]["tool_id"].clone();

    // Apply: add createUser, delete deleteUser.
    let (st, res) = h
        .send(
            "POST",
            &reimport,
            json!({ "content": spec_v2().to_string(), "add": ["POST /users"], "remove": [delete_id] }),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{res}");
    assert_eq!(res["spec"]["version"], "2.0");
    assert_eq!(res["result"]["updated"].as_array().unwrap().len(), 1);
    assert_eq!(res["result"]["created"][0]["name"], "createUser");

    let get_user = h.tool("getUser").await;
    assert_eq!(get_user["description"], "My hand-written description.");
    let params: Vec<_> = get_user["kind"]["params"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].clone())
        .collect();
    assert_eq!(params, [json!("id"), json!("expand")]);
    assert_eq!(get_user["id"], tid, "same tool, updated in place");
    assert_eq!(h.tool("deleteUser").await, Value::Null);
    assert_eq!(h.tool("createUser").await["source"]["spec_id"], spec_id);

    // Re-importing the same spec again changes nothing.
    let (_, again) = h
        .send(
            "POST",
            &format!("{reimport}/preview"),
            json!({ "content": spec_v2().to_string() }),
        )
        .await;
    assert_eq!(again["changed"], json!([]));
    assert_eq!(again["added"], json!([]));
    assert_eq!(again["removed"], json!([]));
    assert_eq!(again["unchanged"], 3);

    // Without content, the stored spec is used.
    let (_, stored) = h
        .send("POST", &format!("{reimport}/preview"), json!({}))
        .await;
    assert_eq!(stored["unchanged"], 3, "{stored}");

    // Stop keeping the description → the next re-import takes the spec's.
    let (st, reset) = h
        .send("POST", &format!("/tools/{tid}/overrides/reset"), json!({}))
        .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(reset["overrides"]["fields"], json!([]));
    let (_, pv) = h
        .send("POST", &format!("{reimport}/preview"), json!({}))
        .await;
    assert_eq!(pv["changed"][0]["fields"], json!(["description"]));
    let (st, _) = h.send("POST", &reimport, json!({})).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(
        h.tool("getUser").await["description"],
        "Fetch one user (v2)"
    );
}

#[tokio::test]
async fn reimport_reuses_options_and_validates_selection() {
    let h = H::start().await;
    let mut v1 = spec_v1();
    v1["servers"] = json!([{ "url": "/relative" }]);
    let (st, done) = h
        .send(
            "POST",
            "/import/openapi",
            json!({ "content": v1.to_string(), "base_url": "https://chosen.example.com", "operations": [{ "key": "GET /users" }] }),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{done}");
    let spec_id = done["spec"]["id"].as_str().unwrap();
    assert_eq!(
        done["spec"]["options"]["base_url"],
        "https://chosen.example.com"
    );

    // The base URL chosen at import is reused, so nothing is blocked.
    let (_, pv) = h
        .send(
            "POST",
            &format!("/specs/{spec_id}/reimport/preview"),
            json!({}),
        )
        .await;
    assert_eq!(pv["base_url"], "https://chosen.example.com");
    assert_eq!(pv["unchanged"], 1);
    assert_eq!(pv["blocked"], json!([]));
    // The other two operations show up as new.
    assert_eq!(pv["added"].as_array().unwrap().len(), 2);

    let (st, e) = h
        .send(
            "POST",
            &format!("/specs/{spec_id}/reimport"),
            json!({ "add": ["GET /nope"] }),
        )
        .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(e["error"]["fields"][0]["field"], "add[0]");
    let (st, e) = h
        .send(
            "POST",
            &format!("/specs/{spec_id}/reimport"),
            json!({ "remove": [uuid_like()] }),
        )
        .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
}

fn uuid_like() -> &'static str {
    "00000000-0000-0000-0000-000000000000"
}
