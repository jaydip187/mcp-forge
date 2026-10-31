//! Stage 24: a GraphQL endpoint imported by introspection, then called over MCP.

use std::sync::Arc;

use forge_exec_http::MapSecretResolver;
use forge_store::Store;
use mcp_forge::{AppState, Security};
use rmcp::model::{CallToolRequestParams, ContentBlock};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::ServiceExt;
use serde_json::{json, Map, Value};
use tokio::net::TcpListener;
use wiremock::matchers::{body_partial_json, body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn t(kind: &str, name: &str) -> Value {
    json!({ "kind": kind, "name": name, "ofType": null })
}

fn nn(inner: Value) -> Value {
    json!({ "kind": "NON_NULL", "name": null, "ofType": inner })
}

fn introspection() -> Value {
    let login = json!({ "name": "login", "type": nn(t("SCALAR", "String")), "defaultValue": null });
    json!({ "data": { "__schema": {
        "queryType": { "name": "Query" },
        "mutationType": { "name": "Mutation" },
        "types": [
            { "kind": "OBJECT", "name": "Query", "fields": [
                { "name": "user", "description": "A user by login.", "args": [login], "type": t("OBJECT", "User") }
            ] },
            { "kind": "OBJECT", "name": "Mutation", "fields": [
                { "name": "follow", "args": [login], "type": t("OBJECT", "User") }
            ] },
            { "kind": "OBJECT", "name": "User", "fields": [
                { "name": "login", "args": [], "type": nn(t("SCALAR", "String")) },
                { "name": "name", "args": [], "type": t("SCALAR", "String") }
            ] },
            { "kind": "SCALAR", "name": "String" }
        ]
    } } })
}

fn call(name: &str, args: Value) -> CallToolRequestParams {
    let m: Map<String, Value> = args.as_object().cloned().unwrap_or_default();
    CallToolRequestParams::new(name.to_owned()).with_arguments(m)
}

#[tokio::test]
async fn introspect_import_and_call() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_string_contains("IntrospectionQuery"))
        .respond_with(ResponseTemplate::new(200).set_body_json(introspection()))
        .mount(&upstream)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_partial_json(
            json!({ "operationName": "User", "variables": { "login": "octocat" } }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({ "data": { "user": { "login": "octocat", "name": "The Octocat" } } }),
        ))
        .mount(&upstream)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_partial_json(
            json!({ "variables": { "login": "ghost" } }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "user": null },
            "errors": [{ "message": "Could not resolve to a User", "path": ["user"] }]
        })))
        .mount(&upstream)
        .await;

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
    let c = reqwest::Client::new();
    let p: Value = c
        .post(format!("{root}/api/projects"))
        .json(&json!({ "name": "Hub" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let base = format!("{root}/api/projects/{}", p["id"].as_str().unwrap());
    let endpoint = format!("{}/graphql", upstream.uri());

    let preview: Value = c
        .post(format!("{base}/import/openapi/preview"))
        .json(&json!({ "url": endpoint, "graphql": true }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(preview["info"]["kind"], "graphql", "{preview}");
    let ops = preview["operations"].as_array().unwrap();
    assert_eq!(ops.len(), 2);
    assert_eq!(ops[0]["key"], "query user");
    assert_eq!(ops[0]["kind"], "graphql");
    assert_eq!(ops[0]["base_url"], endpoint.as_str());
    assert_eq!(ops[1]["key"], "mutation follow");

    let r = c
        .post(format!("{base}/import/openapi"))
        .json(&json!({
            "content": preview["content"],
            "source_url": endpoint,
            "operations": [{ "key": "query user" }, { "key": "mutation follow" }],
        }))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "{}", r.text().await.unwrap());
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["spec"]["kind"], "graphql");
    assert_eq!(body["created"][0]["kind"]["type"], "graphql");
    assert_eq!(body["created"][0]["kind"]["endpoint"], endpoint.as_str());

    let t = StreamableHttpClientTransport::from_uri(format!("{root}/mcp/hub"));
    let client = ().serve(t).await.unwrap();
    let tools = client.list_all_tools().await.unwrap();
    let user = tools.iter().find(|t| t.name == "user").expect("user tool");
    let ann = user.annotations.clone().unwrap();
    assert_eq!(ann.read_only_hint, Some(true));
    let follow = tools
        .iter()
        .find(|t| t.name == "follow")
        .expect("follow tool");
    assert_eq!(
        follow.annotations.clone().unwrap().destructive_hint,
        Some(true)
    );

    let text = |r: &rmcp::model::CallToolResult| -> String {
        r.content
            .iter()
            .filter_map(|c| match c {
                ContentBlock::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect()
    };
    let ok = client
        .call_tool(call("user", json!({ "login": "octocat" })))
        .await
        .unwrap();
    assert_ne!(ok.is_error, Some(true));
    assert!(text(&ok).contains("The Octocat"), "{}", text(&ok));

    let bad = client
        .call_tool(call("user", json!({ "login": "ghost" })))
        .await
        .unwrap();
    assert_eq!(bad.is_error, Some(true));
    assert!(
        text(&bad).contains("Could not resolve to a User (at user)"),
        "{}",
        text(&bad)
    );

    // Read-only projects hide mutations.
    let r = c
        .put(format!("{base}/settings"))
        .json(&json!({ "read_only": true }))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success());
    let names: Vec<String> = client
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    assert_eq!(names, ["user"]);
    client.cancel().await.unwrap();
}
