//! Stage 28: the description linter and "Improve with Claude" (Anthropic
//! API mocked with wiremock).

use std::sync::Arc;

use forge_exec_http::MapSecretResolver;
use forge_store::Store;
use mcp_forge::{AppState, Security};
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "sk-ant-test-key-123";

#[tokio::test]
async fn lint_and_improve() {
    let anthropic = MockServer::start().await;
    let store = Store::open_in_memory().await.unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root = format!("http://{}", listener.local_addr().unwrap());
    let secrets = MapSecretResolver::default().with("env:ANTHROPIC_API_KEY", KEY);
    tokio::spawn(mcp_forge::serve(
        listener,
        AppState::new(store, Arc::new(secrets))
            .with_anthropic_url(anthropic.uri())
            .with_security(Security {
                require_login: false,
                ..Security::default()
            }),
        std::future::pending(),
    ));
    let c = Client::new();
    let send = |m: &str, p: &str, body: Value| {
        let req = c
            .request(m.parse().unwrap(), format!("{root}/api{p}"))
            .json(&body);
        async move {
            let r = req.send().await.unwrap();
            let st = r.status();
            (st, r.json::<Value>().await.unwrap_or(Value::Null))
        }
    };
    let (_, p) = send("POST", "/projects", json!({ "name": "Lint" })).await;
    let base = format!("/projects/{}", p["id"].as_str().unwrap());
    let (_, good) = send(
        "POST",
        &format!("{base}/tools"),
        json!({ "name": "get_repository", "description": "One repository by owner and name: stars, language and open issues.",
                "kind": { "type": "http", "method": "GET", "base_url": "https://api.example.com", "path_template": "/repos/{owner}/{name}",
                          "params": [
                            { "name": "owner", "location": "path", "required": true, "schema": { "type": "string" }, "description": "Account login", "fixed_value": null },
                            { "name": "name", "location": "path", "required": true, "schema": { "type": "string" }, "description": "Repository name", "fixed_value": null }
                          ] } }),
    )
    .await;
    assert!(good["id"].is_string(), "{good}");
    let (_, bad) = send(
        "POST",
        &format!("{base}/tools"),
        json!({ "name": "get", "kind": { "type": "http", "method": "GET", "base_url": "https://api.example.com", "path_template": "/issues",
                "params": [{ "name": "state", "location": "query", "required": false, "schema": { "type": "string" }, "description": null, "fixed_value": null }] } }),
    )
    .await;
    let bad_id = bad["id"].as_str().unwrap().to_owned();

    let (st, report) = send("GET", &format!("{base}/lint"), Value::Null).await;
    assert_eq!(st, StatusCode::OK, "{report}");
    assert_eq!(report["tools"][0]["name"], "get", "worst first");
    let rules: Vec<&str> = report["tools"][0]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["rule"].as_str().unwrap())
        .collect();
    for r in [
        "missing_description",
        "vague_name",
        "undocumented_param",
        "enum_candidate",
        "long_output_unshaped",
    ] {
        assert!(rules.contains(&r), "{r}: {rules:?}");
    }
    assert_eq!(report["tools"][1]["score"], 100);
    assert_eq!(report["improve_available"], false);
    assert!(report["score"].as_u64().unwrap() < 100);

    // Off until a key reference is configured: no request goes out.
    let (st, e) = send("POST", &format!("{base}/tools/{bad_id}/improve"), json!({})).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{e}");
    let (st, e) = send(
        "PUT",
        &format!("{base}/settings"),
        json!({ "anthropic_key_ref": KEY }),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a raw key is refused: {e}"
    );
    let (st, _) = send(
        "PUT",
        &format!("{base}/settings"),
        json!({ "anthropic_key_ref": "env:ANTHROPIC_API_KEY" }),
    )
    .await;
    assert!(st.is_success());

    let answer = json!({
        "description": "Issues across all repositories, filtered by state. Use to find open work.",
        "params": [{ "name": "state", "description": "open, closed or all" }]
    });
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", KEY))
        .and(header("anthropic-version", "2026-10-21"))
        .and(body_partial_json(json!({
            "model": "claude-opus-5-5",
            "output_config": { "effort": "low", "format": { "type": "json_schema" } }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "msg_1", "type": "message", "role": "assistant", "model": "claude-opus-5-5",
            "stop_reason": "end_turn",
            "content": [{ "type": "text", "text": answer.to_string() }]
        })))
        .up_to_n_times(1)
        .mount(&anthropic)
        .await;
    let (st, out) = send("POST", &format!("{base}/tools/{bad_id}/improve"), json!({})).await;
    assert_eq!(st, StatusCode::OK, "{out}");
    assert_eq!(out["suggested"], answer);
    assert_eq!(out["current"]["description"], "");
    let sent = anthropic.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&sent[0].body).unwrap();
    let user = body["messages"][0]["content"].as_str().unwrap();
    assert!(
        user.contains("vague_name") && user.contains("/issues"),
        "{user}"
    );
    // Never applied by itself.
    let (_, still) = send("GET", &format!("{base}/tools/{bad_id}"), Value::Null).await;
    assert_eq!(still["description"], "");

    // Errors never echo the key; refusals are reported plainly.
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "type": "error", "error": { "type": "authentication_error", "message": format!("invalid x-api-key {KEY}") }
        })))
        .up_to_n_times(1)
        .mount(&anthropic)
        .await;
    let (st, e) = send("POST", &format!("{base}/tools/{bad_id}/improve"), json!({})).await;
    assert_eq!(st, StatusCode::BAD_GATEWAY);
    assert!(!e.to_string().contains(KEY), "{e}");
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "msg_2", "type": "message", "role": "assistant", "model": "claude-opus-5-5",
            "stop_reason": "refusal", "content": []
        })))
        .mount(&anthropic)
        .await;
    let (st, e) = send("POST", &format!("{base}/tools/{bad_id}/improve"), json!({})).await;
    assert_eq!(st, StatusCode::BAD_GATEWAY);
    assert!(e.to_string().contains("declined"), "{e}");
    let (_, report) = send("GET", &format!("{base}/lint"), Value::Null).await;
    assert_eq!(report["improve_available"], true);
}
