//! Stage 27: chained tools: argument mapping, a failing middle step, the
//! call log links, cycle rejection, dry runs and MCP annotations.

use std::sync::Arc;

use forge_exec_http::MapSecretResolver;
use forge_store::Store;
use mcp_forge::{AppState, Security};
use reqwest::{Client, StatusCode};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::ServiceExt;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn http_tool(name: &str, method: &str, base: &str, path: &str, params: Value) -> Value {
    json!({ "name": name, "kind": {
        "type": "http", "method": method, "base_url": base, "path_template": path, "params": params
    } })
}

fn path_param(name: &str) -> Value {
    json!({ "name": name, "location": "path", "required": true, "schema": { "type": "string" },
            "description": null, "fixed_value": null })
}

#[tokio::test]
async fn chained_tools() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/users/ada"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "login": "ada", "id": 1 })))
        .mount(&api)
        .await;
    Mock::given(method("GET"))
        .and(path("/users/ada/repos"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([{ "name": "engine" }, { "name": "notes" }])),
        )
        .mount(&api)
        .await;
    Mock::given(method("GET"))
        .and(path("/users/ghost"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({ "message": "Not Found" })))
        .mount(&api)
        .await;
    Mock::given(method("POST"))
        .and(path("/issues"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": 9 })))
        .expect(0)
        .mount(&api)
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
    let (_, p) = send("POST", "/projects", json!({ "name": "Hub" })).await;
    let base = format!("/projects/{}", p["id"].as_str().unwrap());
    let uri = api.uri();
    for t in [
        http_tool(
            "get_user",
            "GET",
            &uri,
            "/users/{login}",
            json!([path_param("login")]),
        ),
        http_tool(
            "list_repos",
            "GET",
            &uri,
            "/users/{owner}/repos",
            json!([path_param("owner")]),
        ),
        http_tool("create_issue", "POST", &uri, "/issues", json!([])),
    ] {
        let (st, e) = send("POST", &format!("{base}/tools"), t).await;
        assert_eq!(st, StatusCode::CREATED, "{e}");
    }

    let chain = json!({ "name": "user_overview", "kind": {
        "type": "chain",
        "params": [{ "name": "login", "required": true }],
        "steps": [
            { "tool": "get_user", "args": "{login: .input.login}" },
            { "tool": "list_repos", "args": "{owner: .last.login}" }
        ],
        "output": "{user: .steps[0].login, repos: [.steps[1][].name]}"
    } });
    let (st, made) = send("POST", &format!("{base}/tools"), chain).await;
    assert_eq!(st, StatusCode::CREATED, "{made}");
    let cid = made["id"].as_str().unwrap().to_owned();

    // Mapping: inputs → step 1 → step 2 → output.
    let (_, run) = send(
        "POST",
        &format!("{base}/tools/{cid}/test"),
        json!({ "arguments": { "login": "ada" } }),
    )
    .await;
    assert_eq!(run["kind"], "chain");
    assert_eq!(run["agent_view"]["is_error"], false, "{run}");
    assert_eq!(
        serde_json::from_str::<Value>(run["agent_view"]["text"].as_str().unwrap()).unwrap(),
        json!({ "user": "ada", "repos": ["engine", "notes"] })
    );
    assert_eq!(
        run["response"]["steps"][1]["args"],
        json!({ "owner": "ada" })
    );

    // A failing first step stops the chain before the second.
    let (_, run) = send(
        "POST",
        &format!("{base}/tools/{cid}/test"),
        json!({ "arguments": { "login": "ghost" } }),
    )
    .await;
    assert_eq!(run["agent_view"]["is_error"], true);
    let text = run["agent_view"]["text"].as_str().unwrap();
    assert!(
        text.starts_with("Step 1 of 2 (`get_user`) failed"),
        "{text}"
    );
    assert_eq!(run["response"]["steps"].as_array().unwrap().len(), 1);

    // Step calls in the log point at the chain call.
    let (_, calls) = send("GET", &format!("{base}/calls?limit=50"), Value::Null).await;
    let calls = calls.get("calls").cloned().unwrap_or(calls);
    let chain_call = calls
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["tool_name"] == "user_overview")
        .unwrap()
        .clone();
    let steps: Vec<&Value> = calls
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["parent_id"] == chain_call["id"])
        .collect();
    assert!(!steps.is_empty(), "{calls}");

    // Cycles and self-calls are refused when saving.
    let (st, e) = send(
        "POST",
        &format!("{base}/tools"),
        json!({ "name": "loop_a", "kind": { "type": "chain", "steps": [{ "tool": "loop_a" }] } }),
    )
    .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    let (st, a) = send(
        "POST",
        &format!("{base}/tools"),
        json!({ "name": "loop_a", "kind": { "type": "chain", "steps": [{ "tool": "get_user", "args": "{login: \"ada\"}" }] } }),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{a}");
    let (st, _) = send(
        "POST",
        &format!("{base}/tools"),
        json!({ "name": "loop_b", "kind": { "type": "chain", "steps": [{ "tool": "loop_a" }] } }),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let (st, e) = send(
        "PUT",
        &format!("{base}/tools/{}", a["id"].as_str().unwrap()),
        json!({ "name": "loop_a", "kind": { "type": "chain", "steps": [{ "tool": "loop_b" }] } }),
    )
    .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    assert!(e.to_string().contains("loop"), "{e}");
    let (st, e) = send(
        "POST",
        &format!("{base}/tools"),
        json!({ "name": "bad_jq", "kind": { "type": "chain", "steps": [{ "tool": "get_user", "args": "{login: .input." }] } }),
    )
    .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    let (st, e) = send(
        "POST",
        &format!("{base}/tools"),
        json!({ "name": "no_such", "kind": { "type": "chain", "steps": [{ "tool": "nope" }] } }),
    )
    .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{e}");

    // A chain with an HTTP write: dry_run skips it; annotations say it writes.
    let (st, w) = send(
        "POST",
        &format!("{base}/tools"),
        json!({ "name": "report_user", "kind": { "type": "chain",
            "params": [{ "name": "login", "required": true }],
            "steps": [
                { "tool": "get_user", "args": "{login: .input.login}" },
                { "tool": "create_issue", "args": "{}" }
            ] } }),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{w}");
    let (_, run) = send(
        "POST",
        &format!("{base}/tools/{}/test", w["id"].as_str().unwrap()),
        json!({ "arguments": { "login": "ada", "dry_run": true } }),
    )
    .await;
    assert_eq!(run["agent_view"]["is_error"], false, "{run}");
    assert_eq!(run["response"]["steps"][1]["skipped"], true);

    let t = StreamableHttpClientTransport::from_uri(format!("{root}/mcp/hub"));
    let client = ().serve(t).await.unwrap();
    let tools = client.list_all_tools().await.unwrap();
    let r = tools.iter().find(|t| t.name == "report_user").unwrap();
    assert_eq!(r.annotations.clone().unwrap().destructive_hint, Some(true));
    assert!(r.input_schema["properties"].get("dry_run").is_some());
    let o = tools.iter().find(|t| t.name == "user_overview").unwrap();
    assert_eq!(o.annotations.clone().unwrap().read_only_hint, Some(true));
    assert!(o
        .description
        .as_deref()
        .unwrap_or("")
        .contains("get_user → list_repos"));
    // Read-only projects hide chains that write.
    send(
        "PUT",
        &format!("{base}/settings"),
        json!({ "read_only": true }),
    )
    .await;
    let names: Vec<String> = client
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    assert!(names.contains(&"user_overview".to_owned()));
    assert!(!names.contains(&"report_user".to_owned()), "{names:?}");
    client.cancel().await.unwrap();
}
