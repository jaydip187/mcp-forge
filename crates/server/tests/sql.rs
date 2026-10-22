//! Stage 8: DB connections and curated read-only SQL tools through the REST
//! API, the test console, MCP and tools.yaml.

use std::str::FromStr;
use std::sync::Arc;

use forge_exec_http::MapSecretResolver;
use forge_store::Store;
use mcp_forge::AppState;
use reqwest::{Client, StatusCode};
use rmcp::model::{CallToolRequestParams, ContentBlock};
use rmcp::ServiceExt;
use serde_json::{json, Map, Value};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, Connection, Executor};
use tokio::net::TcpListener;

struct H {
    root: String,
    c: Client,
    _dir: tempfile::TempDir,
}

impl H {
    async fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shop.db");
        let mut db = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))
            .unwrap()
            .create_if_missing(true)
            .connect()
            .await
            .unwrap();
        db.execute(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, email TEXT);
             INSERT INTO users VALUES (1, 'ada', 'ada@x.io'), (2, 'bob', 'bob@x.io');
             CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER NOT NULL, total REAL);
             INSERT INTO orders VALUES (10, 1, 9.5), (11, 1, 20), (12, 2, 5);
             CREATE TABLE secrets (id INTEGER PRIMARY KEY, token TEXT);
             INSERT INTO secrets VALUES (1, 'shh');",
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
            AppState::new(store, Arc::new(secrets)),
            std::future::pending(),
        ));
        Self {
            root,
            c: Client::new(),
            _dir: dir,
        }
    }

    async fn send(&self, method: &str, p: &str, body: Value) -> (StatusCode, Value) {
        let r = self
            .c
            .request(method.parse().unwrap(), format!("{}/api{p}", self.root))
            .json(&body)
            .send()
            .await
            .unwrap();
        let st = r.status();
        (st, r.json().await.unwrap_or(Value::Null))
    }

    async fn ok(&self, method: &str, p: &str, body: Value) -> Value {
        let (st, v) = self.send(method, p, body).await;
        assert!(st.is_success(), "{method} {p}: {st} {v}");
        v
    }
}

fn sql_tool(name: &str, conn: &str, sql: &str, params: Value) -> Value {
    json!({
        "name": name,
        "description": "Look up users",
        "kind": { "type": "sql", "connection_id": conn,
                  "mode": { "type": "query", "sql": sql, "params": params } }
    })
}

fn text(r: &rmcp::model::CallToolResult) -> String {
    r.content
        .iter()
        .filter_map(|c| match c {
            ContentBlock::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn sql_tool_via_api_console_and_mcp() {
    let h = H::start().await;
    let p = h.ok("POST", "/projects", json!({ "name": "Shop" })).await;
    let base = format!("/projects/{}", p["id"].as_str().unwrap());

    // Raw DSNs are refused; references are stored.
    let (st, _) = h
        .send(
            "POST",
            &format!("{base}/db-connections"),
            json!({ "name": "shop", "driver": "sqlite", "dsn_ref": "/tmp/shop.db" }),
        )
        .await;
    assert!(st.is_client_error(), "{st}");
    let (st, e) = h
        .send(
            "POST",
            &format!("{base}/db-connections"),
            json!({ "name": "shop", "driver": "sqlite", "dsn_ref": "env:SHOP_DB", "access_mode": "full_access" }),
        )
        .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    let conn = h
        .ok(
            "POST",
            &format!("{base}/db-connections"),
            json!({ "name": "shop", "driver": "sqlite", "dsn_ref": "env:SHOP_DB",
                    "column_masks": ["users.email"] }),
        )
        .await;
    let cid = conn["id"].as_str().unwrap();
    assert_eq!(conn["row_limit"], 100);
    assert_eq!(conn["column_masks"], json!(["users.email"]));

    let report = h
        .ok(
            "POST",
            &format!("{base}/db-connections/{cid}/test"),
            json!({}),
        )
        .await;
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(report["can_write"], true, "temp file is writable");
    assert!(!report["warnings"].as_array().unwrap().is_empty());

    // Save-time checks: read-only statement, placeholders match params.
    for (sql, params, field) in [
        ("DELETE FROM users", json!([]), "kind.mode.sql"),
        (
            "SELECT * FROM users WHERE id = :id",
            json!([]),
            "kind.mode.params",
        ),
        (
            "SELECT 1",
            json!([{ "name": "unused" }]),
            "kind.mode.params[0].name",
        ),
    ] {
        let (st, e) = h
            .send(
                "POST",
                &format!("{base}/tools"),
                sql_tool("bad", cid, sql, params),
            )
            .await;
        assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{sql}: {e}");
        assert!(
            e["error"]["fields"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f["field"] == field),
            "{sql}: {e}"
        );
    }
    let (st, e) = h
        .send(
            "POST",
            &format!("{base}/tools"),
            sql_tool(
                "bad",
                "00000000-0000-0000-0000-000000000000",
                "SELECT 1",
                json!([]),
            ),
        )
        .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{e}");

    let tool = h
        .ok(
            "POST",
            &format!("{base}/tools"),
            sql_tool(
                "find_user",
                cid,
                "SELECT id, name, email FROM users WHERE id = :id",
                json!([{ "name": "id", "schema": { "type": "integer" }, "required": true, "description": "User id" }]),
            ),
        )
        .await;
    let tid = tool["id"].as_str().unwrap();

    // Test console shows the statement sent, the rows, and the agent's text.
    let run = h
        .ok(
            "POST",
            &format!("{base}/tools/{tid}/test"),
            json!({ "arguments": { "id": 1 } }),
        )
        .await;
    assert_eq!(run["kind"], "sql");
    assert_eq!(
        run["request"]["sql"],
        "SELECT id, name, email FROM users WHERE id = ?"
    );
    assert_eq!(run["request"]["params"], json!([1]));
    assert_eq!(run["response"]["rows"], json!([[1, "ada", "[masked]"]]));
    assert_eq!(run["response"]["masked_columns"], json!(["email"]));
    let agent: Value = serde_json::from_str(run["agent_view"]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        agent,
        json!({ "columns": ["id", "name", "email"], "rows": [[1, "ada", "[masked]"]], "truncated": false })
    );
    let run = h
        .ok(
            "POST",
            &format!("{base}/tools/{tid}/test"),
            json!({ "arguments": { "id": "one" } }),
        )
        .await;
    assert_eq!(run["error"]["kind"], "invalid_args", "{run}");

    // MCP: listed as read-only (even with the project in read-only mode), callable.
    h.ok(
        "PUT",
        &format!("{base}/settings"),
        json!({ "read_only": true }),
    )
    .await;
    let mcp = ()
        .serve(rmcp::transport::StreamableHttpClientTransport::from_uri(
            format!("{}/mcp/shop", h.root),
        ))
        .await
        .unwrap();
    let tools = mcp.list_all_tools().await.unwrap();
    assert_eq!(tools.len(), 1);
    let ann = tools[0].annotations.as_ref().unwrap();
    assert_eq!(ann.read_only_hint, Some(true));
    assert_eq!(ann.open_world_hint, Some(false));
    assert_eq!(tools[0].input_schema["required"], json!(["id"]));
    let mut args = Map::new();
    args.insert("id".into(), json!(2));
    let r = mcp
        .call_tool(CallToolRequestParams::new("find_user").with_arguments(args))
        .await
        .unwrap();
    assert_ne!(r.is_error, Some(true));
    let v: Value = serde_json::from_str(&text(&r)).unwrap();
    assert_eq!(v["rows"], json!([[2, "bob", "[masked]"]]));

    // A connection in use can't be deleted.
    let (st, _) = h
        .send(
            "DELETE",
            &format!("{base}/db-connections/{cid}"),
            Value::Null,
        )
        .await;
    assert_eq!(st, StatusCode::CONFLICT);
}

#[tokio::test]
async fn bundle_round_trip_with_sql_tools() {
    let h = H::start().await;
    let p = h.ok("POST", "/projects", json!({ "name": "Shop" })).await;
    let base = format!("/projects/{}", p["id"].as_str().unwrap());
    let conn = h
        .ok(
            "POST",
            &format!("{base}/db-connections"),
            json!({ "name": "shop", "driver": "sqlite", "dsn_ref": "env:SHOP_DB", "row_limit": 7 }),
        )
        .await;
    h.ok(
        "POST",
        &format!("{base}/tools"),
        sql_tool(
            "count_users",
            conn["id"].as_str().unwrap(),
            "SELECT count(*) AS n FROM users",
            json!([]),
        ),
    )
    .await;

    let yaml =
        h.c.get(format!("{}/api{base}/export", h.root))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
    assert!(yaml.contains("db_connections:"), "{yaml}");
    assert!(yaml.contains("connection: shop"), "{yaml}");
    assert!(!yaml.contains("connection_id"), "{yaml}");
    assert!(yaml.contains("dsn_ref: env:SHOP_DB"), "{yaml}");

    let p2 = h.ok("POST", "/projects", json!({ "name": "Copy" })).await;
    let base2 = format!("/projects/{}", p2["id"].as_str().unwrap());
    let r = h
        .ok(
            "POST",
            &format!("{base2}/import/bundle"),
            json!({ "content": yaml }),
        )
        .await;
    assert_eq!(r["connections_created"], json!(["shop"]));
    assert_eq!(r["tools_created"], json!(["count_users"]));
    let conns = h
        .ok("GET", &format!("{base2}/db-connections"), Value::Null)
        .await;
    assert_eq!(conns[0]["row_limit"], 7);
    let tools = h.ok("GET", &format!("{base2}/tools"), Value::Null).await;
    assert_eq!(tools[0]["kind"]["connection_id"], conns[0]["id"]);
    let run = h
        .ok(
            "POST",
            &format!("{base2}/tools/{}/test", tools[0]["id"].as_str().unwrap()),
            json!({}),
        )
        .await;
    assert_eq!(run["response"]["rows"], json!([[2]]), "{run}");

    // Writes in a bundle's SQL are rejected before anything is saved.
    let bad = yaml.replace("SELECT count(*) AS n FROM users", "DELETE FROM users");
    let p3 = h.ok("POST", "/projects", json!({ "name": "Bad" })).await;
    let (st, e) = h
        .send(
            "POST",
            &format!("/projects/{}/import/bundle", p3["id"].as_str().unwrap()),
            json!({ "content": bad }),
        )
        .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    assert!(
        e["error"]["message"]
            .as_str()
            .unwrap()
            .contains("tools[0].kind.mode.sql"),
        "{e}"
    );
}

/// Stage 9 "done when": an agent explores the schema and queries within the
/// allowlist; disallowed tables, system catalogs and masked columns are refused.
#[tokio::test]
async fn exploration_tools_respect_allowlist_and_masks() {
    let h = H::start().await;
    let p = h.ok("POST", "/projects", json!({ "name": "Shop" })).await;
    let base = format!("/projects/{}", p["id"].as_str().unwrap());
    let conn = h
        .ok(
            "POST",
            &format!("{base}/db-connections"),
            json!({ "name": "shop", "driver": "sqlite", "dsn_ref": "env:SHOP_DB",
                    "table_allowlist": ["users", "orders"], "column_masks": ["users.email"] }),
        )
        .await;
    let cid = conn["id"].as_str().unwrap();
    let path = format!("{base}/db-connections/{cid}/exploration-tools");
    let (st, e) = h
        .send("POST", &path, json!({ "modes": ["drop_tables"] }))
        .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    let r = h
        .ok(
            "POST",
            &path,
            json!({ "modes": ["list_tables", "describe_table", "run_sql"] }),
        )
        .await;
    let names: Vec<&str> = r["created"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["shop_list_tables", "shop_describe_table", "shop_run_sql"]
    );
    // Opt-in is idempotent.
    let r = h.ok("POST", &path, json!({ "modes": ["run_sql"] })).await;
    assert_eq!(r["created"], json!([]));
    assert_eq!(r["existing"], json!(["shop_run_sql"]));

    let mcp = ()
        .serve(rmcp::transport::StreamableHttpClientTransport::from_uri(
            format!("{}/mcp/shop", h.root),
        ))
        .await
        .unwrap();
    let tools = mcp.list_all_tools().await.unwrap();
    assert_eq!(tools.len(), 3);
    for t in &tools {
        assert_eq!(
            t.annotations.as_ref().unwrap().read_only_hint,
            Some(true),
            "{}",
            t.name
        );
    }
    let run_sql = tools.iter().find(|t| t.name == "shop_run_sql").unwrap();
    let desc = run_sql.description.as_deref().unwrap();
    assert!(
        desc.contains("SQLite") && desc.contains("users, orders") && desc.contains("users.email"),
        "{desc}"
    );

    let call = |name: &'static str, args: Value| {
        let mcp = &mcp;
        async move {
            let r = mcp
                .call_tool(
                    CallToolRequestParams::new(name)
                        .with_arguments(args.as_object().unwrap().clone()),
                )
                .await
                .unwrap();
            (r.is_error == Some(true), text(&r))
        }
    };

    let (err, out) = call("shop_list_tables", json!({})).await;
    assert!(!err, "{out}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        v["rows"],
        json!([["main", "orders", "table"], ["main", "users", "table"]])
    );

    let (err, out) = call("shop_describe_table", json!({ "table": "users" })).await;
    assert!(!err, "{out}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        v["columns"],
        json!([
            "column",
            "type",
            "nullable",
            "default",
            "primary_key",
            "masked"
        ])
    );
    assert_eq!(
        v["rows"][0],
        json!(["id", "INTEGER", true, null, true, false])
    );
    assert_eq!(v["rows"][2][0], "email");
    assert_eq!(v["rows"][2][5], true);

    let (err, out) = call(
        "shop_run_sql",
        json!({ "sql": "SELECT u.name, sum(o.total) AS spent FROM users u JOIN orders o ON o.user_id = u.id GROUP BY u.name ORDER BY u.name" }),
    )
    .await;
    assert!(!err, "{out}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["rows"], json!([["ada", 29.5], ["bob", 5.0]]));

    let (err, out) = call(
        "shop_run_sql",
        json!({ "sql": "SELECT * FROM users WHERE id = 1" }),
    )
    .await;
    assert!(!err, "{out}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["rows"], json!([[1, "ada", "[masked]"]]));

    for (tool, args, why) in [
        (
            "shop_describe_table",
            json!({ "table": "secrets" }),
            "isn't available",
        ),
        (
            "shop_run_sql",
            json!({ "sql": "SELECT * FROM secrets" }),
            "isn't available",
        ),
        (
            "shop_run_sql",
            json!({ "sql": "WITH users AS (SELECT * FROM secrets) SELECT * FROM users" }),
            "isn't available",
        ),
        (
            "shop_run_sql",
            json!({ "sql": "SELECT name FROM sqlite_master" }),
            "system catalog",
        ),
        (
            "shop_run_sql",
            json!({ "sql": "SELECT email FROM users" }),
            "masked",
        ),
        (
            "shop_run_sql",
            json!({ "sql": "SELECT upper(email) AS e FROM users" }),
            "masked",
        ),
        (
            "shop_run_sql",
            json!({ "sql": "DELETE FROM orders" }),
            "only SELECT",
        ),
        (
            "shop_run_sql",
            json!({ "sql": "SELECT * FROM orders WHERE id = ?" }),
            "placeholders",
        ),
    ] {
        let (err, out) = call(tool, args.clone()).await;
        assert!(err, "{tool} {args}: {out}");
        assert!(out.contains(why), "{tool} {args}: {out}");
    }
}

/// Stage 10 "done when": mode gating, WHERE enforcement, dry run, the
/// affected-rows cap and audit entries, end to end.
#[tokio::test]
async fn writes_are_gated_capped_and_audited() {
    let h = H::start().await;
    let p = h.ok("POST", "/projects", json!({ "name": "Shop" })).await;
    let base = format!("/projects/{}", p["id"].as_str().unwrap());

    // full_access needs explicit confirmation, on create and on update.
    let full = json!({ "name": "shop", "driver": "sqlite", "dsn_ref": "env:SHOP_DB",
                       "access_mode": "full_access", "max_affected_rows": 1 });
    let (st, e) = h
        .send("POST", &format!("{base}/db-connections"), full.clone())
        .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    assert_eq!(e["error"]["fields"][0]["field"], "access_mode");
    let conn = h
        .ok(
            "POST",
            &format!("{base}/db-connections"),
            json!({ "name": "shop", "driver": "sqlite", "dsn_ref": "env:SHOP_DB",
                    "access_mode": "curated_write", "max_affected_rows": 1 }),
        )
        .await;
    let cid = conn["id"].as_str().unwrap();

    // Save-time: WHERE is required; DDL is off; admin statements never pass.
    for (sql, why) in [
        ("DELETE FROM orders", "WHERE"),
        ("DROP TABLE orders", "schema changes"),
        ("PRAGMA writable_schema = 1", "never allowed"),
    ] {
        let (st, e) = h
            .send(
                "POST",
                &format!("{base}/tools"),
                sql_tool("bad", cid, sql, json!([])),
            )
            .await;
        assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{sql}: {e}");
        assert!(e.to_string().contains(why), "{sql}: {e}");
    }
    h.ok(
        "POST",
        &format!("{base}/tools"),
        sql_tool(
            "set_total",
            cid,
            "UPDATE orders SET total = :total WHERE id = :id",
            json!([{ "name": "id", "schema": { "type": "integer" }, "required": true },
                   { "name": "total", "schema": { "type": "number" }, "required": true }]),
        ),
    )
    .await;
    let bulk = h
        .ok(
            "POST",
            &format!("{base}/tools"),
            sql_tool(
                "zero_user_orders",
                cid,
                "UPDATE orders SET total = 0 WHERE user_id = :user_id",
                json!([{ "name": "user_id", "schema": { "type": "integer" }, "required": true }]),
            ),
        )
        .await;
    h.ok(
        "POST",
        &format!("{base}/db-connections/{cid}/exploration-tools"),
        json!({ "modes": ["run_sql"] }),
    )
    .await;

    let mcp = ()
        .serve(rmcp::transport::StreamableHttpClientTransport::from_uri(
            format!("{}/mcp/shop", h.root),
        ))
        .await
        .unwrap();
    let tools = mcp.list_all_tools().await.unwrap();
    let tool = |n: &str| tools.iter().find(|t| t.name == n).unwrap().clone();
    let set_total = tool("set_total");
    let ann = set_total.annotations.as_ref().unwrap();
    assert_eq!(
        (ann.read_only_hint, ann.destructive_hint),
        (Some(false), Some(true))
    );
    assert!(set_total.input_schema["properties"]["dry_run"].is_object());
    assert!(set_total
        .description
        .as_deref()
        .unwrap()
        .contains("dry_run"));
    // run_sql stays read-only under curated_write.
    let run_sql = tool("shop_run_sql");
    assert_eq!(
        run_sql.annotations.as_ref().unwrap().read_only_hint,
        Some(true)
    );
    assert!(run_sql.input_schema["properties"].get("dry_run").is_none());

    let call = |name: &'static str, args: Value| {
        let mcp = &mcp;
        async move {
            let r = mcp
                .call_tool(
                    CallToolRequestParams::new(name)
                        .with_arguments(args.as_object().unwrap().clone()),
                )
                .await
                .unwrap();
            (r.is_error == Some(true), text(&r))
        }
    };
    let (err, out) = call(
        "set_total",
        json!({ "id": 10, "total": 1.5, "dry_run": true }),
    )
    .await;
    assert!(!err, "{out}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        (v["affected_rows"].clone(), v["committed"].clone()),
        (json!(1), json!(false))
    );
    let (err, out) = call("set_total", json!({ "id": 10, "total": 1.5 })).await;
    assert!(!err, "{out}");
    // user 1 has two orders; the cap is 1.
    let (err, out) = call("zero_user_orders", json!({ "user_id": 1 })).await;
    assert!(
        err && out.contains("Not done") && out.contains("nothing was changed"),
        "{out}"
    );
    let (err, out) = call(
        "shop_run_sql",
        json!({ "sql": "DELETE FROM orders WHERE id = 12" }),
    )
    .await;
    assert!(err && out.contains("saved tools"), "{out}");

    // The console runs the same path and is audited as "console".
    let run = h
        .ok(
            "POST",
            &format!("{base}/tools/{}/test", bulk["id"].as_str().unwrap()),
            json!({ "arguments": { "user_id": 2 } }),
        )
        .await;
    assert_eq!(run["response"]["write"]["affected_rows"], 1, "{run}");

    let check = h
        .ok(
            "POST",
            &format!("{base}/tools/{}/test", bulk["id"].as_str().unwrap()),
            json!({ "arguments": {} }),
        )
        .await;
    assert_eq!(check["error"]["kind"], "invalid_args");

    let audit = h.ok("GET", &format!("{base}/audit"), Value::Null).await;
    let rows: Vec<(String, String, String, Value)> = audit
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["tool_name"].as_str().unwrap().to_owned(),
                e["origin"].as_str().unwrap().to_owned(),
                e["outcome"].as_str().unwrap().to_owned(),
                e["rows_affected"].clone(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            (
                "zero_user_orders".into(),
                "console".into(),
                "committed".into(),
                json!(1)
            ),
            (
                "zero_user_orders".into(),
                "mcp".into(),
                "rolled_back".into(),
                json!(2)
            ),
            (
                "set_total".into(),
                "mcp".into(),
                "committed".into(),
                json!(1)
            ),
            ("set_total".into(), "mcp".into(), "dry_run".into(), json!(1)),
        ],
        "rejected and invalid calls aren't audited; newest first"
    );
    assert_eq!(audit[2]["params"], json!([1.5, 10]));
    assert_eq!(audit[2]["sql"], "UPDATE orders SET total = ? WHERE id = ?");

    // A read-only project hides tools that write.
    h.ok(
        "PUT",
        &format!("{base}/settings"),
        json!({ "read_only": true }),
    )
    .await;
    let names: Vec<String> = mcp
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    assert_eq!(names, ["shop_run_sql"]);

    // Upgrading to full_access: confirmed, then run_sql writes (and gets dry_run).
    let (st, _) = h
        .send("PUT", &format!("{base}/db-connections/{cid}"), full.clone())
        .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
    let mut confirmed = full;
    confirmed["confirm_full_access"] = json!(true);
    h.ok("PUT", &format!("{base}/db-connections/{cid}"), confirmed)
        .await;
    h.ok(
        "PUT",
        &format!("{base}/settings"),
        json!({ "read_only": false }),
    )
    .await;
    let (err, out) = call(
        "shop_run_sql",
        json!({ "sql": "DELETE FROM orders WHERE id = 12", "dry_run": true }),
    )
    .await;
    assert!(!err, "{out}");
    assert!(out.contains("\"dry_run\":true"), "{out}");
}
