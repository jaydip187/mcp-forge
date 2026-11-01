//! Stage 25b: a NoSQL connection (Elasticsearch, mocked) through the REST
//! API, the test console, MCP, the audit log and read-only mode.

use std::sync::Arc;

use forge_exec_http::MapSecretResolver;
use forge_store::Store;
use mcp_forge::{AppState, Security};
use reqwest::{Client, StatusCode};
use rmcp::model::{CallToolRequestParams, ContentBlock};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::ServiceExt;
use serde_json::{json, Map, Value};
use tokio::net::TcpListener;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn call(name: &str, args: Value) -> CallToolRequestParams {
    let m: Map<String, Value> = args.as_object().cloned().unwrap_or_default();
    CallToolRequestParams::new(name.to_owned()).with_arguments(m)
}

#[tokio::test]
async fn elasticsearch_tools_end_to_end() {
    let es = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/products/_search"))
        .and(body_partial_json(json!({ "query": { "match": { "title": "lamp" } } })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "hits": { "total": { "value": 1 }, "hits": [
                { "_id": "7", "_index": "products", "_source": { "title": "lamp", "supplier_key": "s3cr3t" } }
            ] }
        })))
        .mount(&es)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/products/_doc/7"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "_id": "7", "result": "deleted" })),
        )
        .expect(1)
        .mount(&es)
        .await;

    let secrets = MapSecretResolver::default().with("env:ES", &es.uri());
    let store = Store::open_in_memory().await.unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(mcp_forge::serve(
        listener,
        AppState::new(store, Arc::new(secrets)).with_security(Security {
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
    let (_, p) = send("POST", "/projects", json!({ "name": "Catalog" })).await;
    let base = format!("/projects/{}", p["id"].as_str().unwrap());
    let (st, conn) = send(
        "POST",
        &format!("{base}/db-connections"),
        json!({ "name": "search", "driver": "elasticsearch", "dsn_ref": "env:ES",
                "access_mode": "curated_write", "table_allowlist": ["products"],
                "column_masks": ["products.supplier_key"] }),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{conn}");
    let cid = conn["id"].as_str().unwrap().to_owned();

    // Save-time checks: unknown operation, disallowed index, scripts.
    for (body, field) in [
        (
            json!({ "operation": "FLUSHALL", "target": "products" }),
            "kind.operation",
        ),
        (
            json!({ "operation": "search", "target": "secrets" }),
            "kind.target",
        ),
        (
            json!({ "operation": "search", "target": "products", "body": { "script_fields": {} } }),
            "kind.body",
        ),
    ] {
        let mut kind = json!({ "type": "nosql", "connection_id": cid });
        kind.as_object_mut()
            .unwrap()
            .extend(body.as_object().unwrap().clone());
        let (st, e) = send(
            "POST",
            &format!("{base}/tools"),
            json!({ "name": "bad", "kind": kind }),
        )
        .await;
        assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
        assert!(e.to_string().contains(field), "{field}: {e}");
    }
    // A SQL tool can't use a NoSQL connection.
    let (st, e) = send(
        "POST",
        &format!("{base}/tools"),
        json!({ "name": "sql", "kind": { "type": "sql", "connection_id": cid, "mode": { "type": "query", "sql": "SELECT 1" } } }),
    )
    .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{e}");

    let (st, find) = send(
        "POST",
        &format!("{base}/tools"),
        json!({ "name": "find_products", "kind": {
            "type": "nosql", "connection_id": cid, "operation": "search", "target": "products",
            "body": { "query": { "match": { "title": "{{q}}" } } },
            "params": [{ "name": "q", "required": true }]
        } }),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{find}");
    let (st, del) = send(
        "POST",
        &format!("{base}/tools"),
        json!({ "name": "delete_product", "kind": {
            "type": "nosql", "connection_id": cid, "operation": "delete", "target": "products",
            "body": { "id": "{{id}}" }, "params": [{ "name": "id", "required": true }]
        } }),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{del}");

    // Test console: documents with masks applied.
    let (st, run) = send(
        "POST",
        &format!("{base}/tools/{}/test", find["id"].as_str().unwrap()),
        json!({ "arguments": { "q": "lamp" } }),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{run}");
    assert_eq!(run["kind"], "nosql");
    assert_eq!(run["response"]["documents"][0]["supplier_key"], "[masked]");
    assert_eq!(run["request"]["body"]["query"]["match"]["title"], "lamp");
    assert!(!run["agent_view"]["text"]
        .as_str()
        .unwrap()
        .contains("s3cr3t"));

    // MCP: annotations, a write with dry_run in its schema, audited.
    let t = StreamableHttpClientTransport::from_uri(format!("{root}/mcp/catalog"));
    let client = ().serve(t).await.unwrap();
    let tools = client.list_all_tools().await.unwrap();
    let d = tools.iter().find(|t| t.name == "delete_product").unwrap();
    assert_eq!(d.annotations.clone().unwrap().destructive_hint, Some(true));
    assert!(d.input_schema["properties"].get("dry_run").is_some());
    let f = tools.iter().find(|t| t.name == "find_products").unwrap();
    assert_eq!(f.annotations.clone().unwrap().read_only_hint, Some(true));
    let r = client
        .call_tool(call("delete_product", json!({ "id": "7" })))
        .await
        .unwrap();
    let text: String = r
        .content
        .iter()
        .filter_map(|c| match c {
            ContentBlock::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect();
    assert_ne!(r.is_error, Some(true), "{text}");
    assert!(text.contains("\"affected\":1"), "{text}");
    let (_, audit) = send("GET", &format!("{base}/audit"), json!(null)).await;
    let entry = &audit[0];
    assert_eq!(entry["tool_name"], "delete_product", "{audit}");
    assert_eq!(entry["outcome"], "committed");

    // Agent tools for the connection, then read-only hides the write.
    let (st, made) = send(
        "POST",
        &format!("{base}/db-connections/{cid}/exploration-tools"),
        json!({ "modes": ["list_targets", "describe_target", "run_query"] }),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{made}");
    assert_eq!(made["created"].as_array().unwrap().len(), 3);
    let (st, _) = send(
        "PUT",
        &format!("{base}/settings"),
        json!({ "read_only": true }),
    )
    .await;
    assert!(st.is_success());
    let names: Vec<String> = client
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    assert!(names.contains(&"find_products".to_owned()));
    assert!(!names.contains(&"delete_product".to_owned()), "{names:?}");
    client.cancel().await.unwrap();
}
