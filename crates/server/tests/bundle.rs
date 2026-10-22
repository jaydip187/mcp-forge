//! tools.yaml export/import via the API and the CLI.

use std::process::Stdio;
use std::sync::Arc;

use forge_core::{AuthConfigInput, ProjectInput, ToolInput};
use forge_exec_http::MapSecretResolver;
use forge_store::Store;
use mcp_forge::AppState;
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::process::Command;

fn tool_json(name: &str) -> Value {
    json!({
        "name": name,
        "description": "Fetch a user",
        "kind": {
            "type": "http", "method": "GET", "base_url": "https://api.example.com",
            "path_template": "/users/{id}",
            "params": [
                { "name": "id", "location": "path", "required": true },
                { "name": "filter_name", "wire_name": "filter[name]", "location": "query" },
                { "name": "X-Version", "location": "header", "fixed_value": "2" }
            ]
        }
    })
}

async fn post(c: &Client, url: String, body: Value) -> (StatusCode, Value) {
    let r = c.post(url).json(&body).send().await.unwrap();
    let st = r.status();
    (st, r.json().await.unwrap_or(Value::Null))
}

#[tokio::test]
async fn api_round_trip() {
    let store = Store::open_in_memory().await.unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/api", listener.local_addr().unwrap());
    tokio::spawn(mcp_forge::serve(
        listener,
        AppState::new(store, Arc::new(MapSecretResolver::default())),
        std::future::pending(),
    ));
    let c = Client::new();
    let (_, p1) = post(&c, format!("{base}/projects"), json!({ "name": "Source" })).await;
    let p1 = p1["id"].as_str().unwrap();
    let (_, auth) = post(
        &c,
        format!("{base}/projects/{p1}/auth-configs"),
        json!({ "name": "gh", "kind": { "type": "bearer" }, "secret_ref": "env:GH_TOKEN" }),
    )
    .await;
    let mut with_auth = tool_json("get_user");
    with_auth["kind"]["auth_ref"] = auth["id"].clone();
    let (st, _) = post(&c, format!("{base}/projects/{p1}/tools"), with_auth).await;
    assert_eq!(st, StatusCode::CREATED);
    let (st, _) = post(
        &c,
        format!("{base}/projects/{p1}/tools"),
        tool_json("plain"),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);

    let r = c
        .get(format!("{base}/projects/{p1}/export"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert!(r.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("application/yaml"));
    assert!(r.headers()["content-disposition"]
        .to_str()
        .unwrap()
        .contains("source-tools.yaml"));
    let yaml = r.text().await.unwrap();
    assert!(yaml.contains("secret_ref: env:GH_TOKEN"), "{yaml}");
    assert!(yaml.contains("auth: gh"), "{yaml}");
    assert!(
        !yaml.contains(auth["id"].as_str().unwrap()),
        "ids never leave the machine"
    );
    assert!(yaml.contains("wire_name: filter[name]"));

    // Import into a second project; exporting it again gives the same tools.
    let (_, p2) = post(&c, format!("{base}/projects"), json!({ "name": "Copy" })).await;
    let p2 = p2["id"].as_str().unwrap();
    let (st, report) = post(
        &c,
        format!("{base}/projects/{p2}/import/bundle"),
        json!({ "content": yaml }),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{report}");
    assert_eq!(report["tools_created"], json!(["get_user", "plain"]));
    assert_eq!(report["auth_created"], json!(["gh"]));
    let again = c
        .get(format!("{base}/projects/{p2}/export"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let strip_project = |s: &str| s.split("auth_configs:").nth(1).unwrap().to_owned();
    assert_eq!(strip_project(&again), strip_project(&yaml));

    // Importing again conflicts by default; skip and replace policies work.
    let (st, e) = post(
        &c,
        format!("{base}/projects/{p2}/import/bundle"),
        json!({ "content": yaml }),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT, "{e}");
    let (st, r) = post(
        &c,
        format!("{base}/projects/{p2}/import/bundle"),
        json!({ "content": yaml, "on_conflict": "skip" }),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(r["tools_skipped"].as_array().unwrap().len(), 2);

    // A raw secret in the file is rejected before anything is written.
    let bad = yaml.replace("env:GH_TOKEN", "ghp_rawvalue");
    let (st, e) = post(
        &c,
        format!("{base}/projects/{p2}/import/bundle"),
        json!({ "content": bad }),
    )
    .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    assert!(e["error"]["message"]
        .as_str()
        .unwrap()
        .contains("raw secret"));
}

#[tokio::test]
async fn cli_export_then_import_into_new_project() {
    let tmp = tempfile::tempdir().unwrap();
    {
        let store = Store::open(tmp.path()).await.unwrap();
        let p = store
            .create_project(&ProjectInput {
                name: "Origin".into(),
                slug: None,
            })
            .await
            .unwrap();
        let auth: AuthConfigInput = serde_json::from_value(json!({
            "name": "key", "kind": { "type": "api_key", "location": "header", "name": "X-Key" }, "secret_ref": "keyring:my-api"
        }))
        .unwrap();
        let a = store.create_auth_config(p.id, &auth).await.unwrap();
        let mut t: ToolInput = serde_json::from_value(tool_json("get_user")).unwrap();
        let forge_core::ToolKind::Http(h) = &mut t.kind else {
            panic!("expected an HTTP tool")
        };
        h.auth_ref = Some(a.id);
        store.create_tool(p.id, &t).await.unwrap();
        store.pool().close().await;
    }
    let exe = env!("CARGO_BIN_EXE_mcp-forge");
    let file = tmp.path().join("tools.yaml");

    let out = Command::new(exe)
        .args(["export", "--project", "origin", "--out"])
        .arg(&file)
        .arg("--data-dir")
        .arg(tmp.path())
        .output()
        .await
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let yaml = std::fs::read_to_string(&file).unwrap();
    assert!(
        yaml.contains("secret_ref: keyring:my-api") && yaml.contains("auth: key"),
        "{yaml}"
    );

    // stdout form, as in `mcp-forge export --project x > tools.yaml`.
    let out = Command::new(exe)
        .args(["export", "--project", "origin", "--data-dir"])
        .arg(tmp.path())
        .output()
        .await
        .unwrap();
    assert_eq!(String::from_utf8(out.stdout).unwrap(), yaml);

    let out = Command::new(exe)
        .args(["import", "--project", "copy"])
        .arg(&file)
        .arg("--data-dir")
        .arg(tmp.path())
        .stdin(Stdio::null())
        .output()
        .await
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(
        stderr.contains("Created project `copy`") && stderr.contains("tools 1 created"),
        "{stderr}"
    );

    let store = Store::open(tmp.path()).await.unwrap();
    let copy = store.get_project_by_slug("copy").await.unwrap();
    assert_eq!(copy.name, "Origin");
    let tools = store.list_tools(copy.id).await.unwrap();
    let auths = store.list_auth_configs(copy.id).await.unwrap();
    let forge_core::ToolKind::Http(h) = &tools[0].kind else {
        panic!("expected an HTTP tool")
    };
    assert_eq!(h.auth_ref, Some(auths[0].id));

    // Second import without a policy fails and changes nothing.
    let out = Command::new(exe)
        .args(["import", "--project", "copy"])
        .arg(&file)
        .arg("--data-dir")
        .arg(tmp.path())
        .output()
        .await
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("already exists"));
}
