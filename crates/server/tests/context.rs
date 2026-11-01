//! Stage 26: MCP resources (written, database schema, shared specs) and
//! prompts, through the REST API and an MCP client.

use std::str::FromStr;
use std::sync::Arc;

use forge_exec_http::MapSecretResolver;
use forge_store::Store;
use mcp_forge::{AppState, Security};
use reqwest::{Client, StatusCode};
use rmcp::model::{GetPromptRequestParams, ReadResourceRequestParams, ResourceContents};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::ServiceExt;
use serde_json::{json, Value};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, Connection, Executor};
use tokio::net::TcpListener;

async fn uris(c: &rmcp::service::RunningService<rmcp::RoleClient, ()>) -> Vec<String> {
    c.list_all_resources()
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.uri.clone())
        .collect()
}

fn text(c: &ResourceContents) -> String {
    match c {
        ResourceContents::TextResourceContents { text, .. } => text.clone(),
        _ => String::new(),
    }
}

#[tokio::test]
async fn resources_and_prompts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shop.db");
    let mut db = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))
        .unwrap()
        .create_if_missing(true)
        .connect()
        .await
        .unwrap();
    db.execute(
        "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, password_hash TEXT);
         CREATE TABLE secrets (id INTEGER PRIMARY KEY, token TEXT);",
    )
    .await
    .unwrap();
    db.close().await.unwrap();

    let secrets = MapSecretResolver::default().with("env:SHOP_DB", &path.display().to_string());
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
    let (_, p) = send("POST", "/projects", json!({ "name": "Shop" })).await;
    let base = format!("/projects/{}", p["id"].as_str().unwrap());

    // Validation.
    let (st, e) = send(
        "POST",
        &format!("{base}/resources"),
        json!({ "name": "x", "mime_type": "application/json", "content": "{nope" }),
    )
    .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    let (st, e) = send(
        "POST",
        &format!("{base}/prompts"),
        json!({ "name": "p", "template": "Hi {{who}}", "arguments": [] }),
    )
    .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{e}");

    let (st, r) = send(
        "POST",
        &format!("{base}/resources"),
        json!({ "name": "style_guide", "description": "How we write", "content": "# Be brief" }),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{r}");
    let (st, pr) = send(
        "POST",
        &format!("{base}/prompts"),
        json!({ "name": "triage", "description": "Triage a ticket",
                "arguments": [{ "name": "ticket", "required": true }, { "name": "tone" }],
                "template": "Look up ticket {{ticket}} and answer in a {{tone}} tone." }),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{pr}");
    let (_, conn) = send(
        "POST",
        &format!("{base}/db-connections"),
        json!({ "name": "shop", "driver": "sqlite", "dsn_ref": "env:SHOP_DB",
                "table_allowlist": ["users"], "column_masks": ["users.password_hash"] }),
    )
    .await;
    let cid = conn["id"].as_str().unwrap();

    let t = StreamableHttpClientTransport::from_uri(format!("{root}/mcp/shop"));
    let client = ().serve(t).await.unwrap();
    assert_eq!(
        uris(&client).await,
        ["forge://resources/style_guide"],
        "no schema until the describe tool exists"
    );

    let read = client
        .read_resource(ReadResourceRequestParams::new(
            "forge://resources/style_guide",
        ))
        .await
        .unwrap();
    assert_eq!(text(&read.contents[0]), "# Be brief");

    // Turning on the describe tool offers the schema (allowlist applies).
    let (st, _) = send(
        "POST",
        &format!("{base}/db-connections/{cid}/exploration-tools"),
        json!({ "modes": ["list_tables", "describe_table"] }),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    assert!(uris(&client).await.contains(&"schema://shop".to_owned()));
    let schema = client
        .read_resource(ReadResourceRequestParams::new("schema://shop"))
        .await
        .unwrap();
    let md = text(&schema.contents[0]);
    assert!(
        md.contains("## main.users") && md.contains("password_hash"),
        "{md}"
    );
    assert!(!md.contains("secrets"), "allowlist applies: {md}");
    assert!(client
        .read_resource(ReadResourceRequestParams::new("schema://other"))
        .await
        .is_err());

    // Specs only when shared.
    let (st, imported) = send(
        "POST",
        &format!("{base}/import/openapi"),
        json!({ "content": "{\"openapi\":\"3.0.0\",\"info\":{\"title\":\"Pets\",\"version\":\"1\"},\"servers\":[{\"url\":\"https://p.example\"}],\"paths\":{\"/pets\":{\"get\":{\"operationId\":\"listPets\",\"responses\":{\"200\":{\"description\":\"ok\"}}}}}}",
                "operations": [{ "key": "GET /pets" }] }),
    )
    .await;
    assert!(st.is_success(), "{imported}");
    let spec_uri = format!("spec://{}", imported["spec"]["id"].as_str().unwrap());
    assert!(!uris(&client).await.contains(&spec_uri));
    send(
        "PUT",
        &format!("{base}/settings"),
        json!({ "share_specs": true }),
    )
    .await;
    assert!(uris(&client).await.contains(&spec_uri));
    let spec = client
        .read_resource(ReadResourceRequestParams::new(spec_uri.clone()))
        .await
        .unwrap();
    assert!(text(&spec.contents[0]).contains("listPets"));

    // Prompts.
    let prompts = client.list_all_prompts().await.unwrap();
    assert_eq!(prompts[0].name, "triage");
    assert_eq!(
        prompts[0].arguments.as_ref().unwrap()[0].required,
        Some(true)
    );
    let got = client
        .get_prompt(
            GetPromptRequestParams::new("triage").with_arguments(
                json!({ "ticket": "#42", "tone": "kind" })
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    let msg = serde_json::to_value(&got.messages[0]).unwrap();
    assert_eq!(
        msg["content"]["text"],
        "Look up ticket #42 and answer in a kind tone."
    );
    assert!(
        client
            .get_prompt(GetPromptRequestParams::new("triage"))
            .await
            .is_err(),
        "ticket is required"
    );

    // The UI preview endpoints.
    let (_, listed) = send("GET", &format!("{base}/mcp-resources"), Value::Null).await;
    assert_eq!(listed.as_array().unwrap().len(), 3, "{listed}");
    let (_, rendered) = send(
        "POST",
        &format!("{base}/prompts/{}/render", pr["id"].as_str().unwrap()),
        json!({ "arguments": { "ticket": "1" } }),
    )
    .await;
    assert_eq!(rendered["text"], "Look up ticket 1 and answer in a  tone.");
    client.cancel().await.unwrap();
}
