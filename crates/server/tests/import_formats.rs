//! Stage 23: HAR / Postman / curl through the import API. The preview hands
//! back the converted OpenAPI document, and the stored spec is that
//! conversion, never the original (which can hold cookies and tokens).

use std::sync::Arc;

use forge_exec_http::MapSecretResolver;
use forge_store::Store;
use mcp_forge::{AppState, Security};
use serde_json::{json, Value};
use tokio::net::TcpListener;

const HAR: &str = r#"{ "log": { "entries": [
  { "request": { "method": "GET", "url": "https://api.example.com/v1/users/42?fields=name",
                 "headers": [{ "name": "Cookie", "value": "sid=SECRET-COOKIE" }, { "name": "Authorization", "value": "Bearer SECRET-TOKEN" }] },
    "response": { "status": 200, "content": { "mimeType": "application/json" } } }
] } }"#;

#[tokio::test]
async fn har_import_keeps_only_the_conversion() {
    let store = Store::open_in_memory().await.unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/api", listener.local_addr().unwrap());
    tokio::spawn(mcp_forge::serve(
        listener,
        AppState::new(store.clone(), Arc::new(MapSecretResolver::default())).with_security(
            Security {
                require_login: false,
                ..Security::default()
            },
        ),
        std::future::pending(),
    ));
    let c = reqwest::Client::new();
    let p: Value = c
        .post(format!("{base}/projects"))
        .json(&json!({ "name": "Captured" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let pid = p["id"].as_str().unwrap();

    let preview: Value = c
        .post(format!("{base}/projects/{pid}/import/openapi/preview"))
        .json(&json!({ "content": HAR }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(preview["info"]["kind"], "har", "{preview}");
    assert_eq!(preview["operations"][0]["key"], "GET /v1/users/{users_id}");
    let converted = preview["content"]
        .as_str()
        .expect("converted document returned");
    assert!(converted.contains("\"openapi\""));
    assert!(!converted.contains("SECRET"), "{converted}");

    // Even when a client commits the original, the stored spec is the conversion.
    let r = c
        .post(format!("{base}/projects/{pid}/import/openapi"))
        .json(&json!({
            "content": HAR,
            "filename": "capture.har",
            "operations": [{ "key": "GET /v1/users/{users_id}" }],
        }))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "{}", r.text().await.unwrap());
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["spec"]["kind"], "har");
    assert_eq!(
        body["created"][0]["kind"]["path_template"],
        "/v1/users/{users_id}"
    );
    let spec_id = body["spec"]["id"].as_str().unwrap().parse().unwrap();
    let raw = store
        .get_spec_raw(pid.parse().unwrap(), spec_id)
        .await
        .unwrap();
    assert!(raw.contains("x-mcp-forge-converted-from"));
    assert!(!raw.contains("SECRET"), "stored: {raw}");

    // A curl command previews the same way.
    let preview: Value = c
        .post(format!("{base}/projects/{pid}/import/openapi/preview"))
        .json(&json!({ "content": "curl https://api.example.com/v1/ping -H 'X-Api-Key: SECRET-KEY'" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(preview["info"]["kind"], "curl");
    assert!(!preview["content"].as_str().unwrap().contains("SECRET"));
}
