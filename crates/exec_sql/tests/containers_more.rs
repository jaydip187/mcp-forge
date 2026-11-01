//! Stage 25: SQL Server and ClickHouse in Docker (testcontainers). Run with
//! `cargo test -p forge-exec-sql --test containers_more -- --ignored`
//! (needs a running Docker daemon; the SQL Server image is large).

mod common;

use common::{args, mode, query, setup};
use forge_exec_sql::{run_read_only, Limits, SqlError, MASKED};
use serde_json::json;
use testcontainers_modules::clickhouse::ClickHouse;
use testcontainers_modules::mssql_server::MssqlServer;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use tokio::net::TcpStream;
use tokio_util::compat::TokioAsyncWriteCompatExt;

const LIMITS: Limits = Limits {
    row_limit: 10,
    timeout_ms: 5000,
};

async fn mssql_admin(port: u16, sql: &str) {
    let mut c = tiberius::Config::new();
    c.host("127.0.0.1");
    c.port(port);
    c.authentication(tiberius::AuthMethod::sql_server(
        "sa",
        MssqlServer::DEFAULT_SA_PASSWORD,
    ));
    c.trust_cert();
    let tcp = TcpStream::connect(c.get_addr()).await.unwrap();
    let mut client = tiberius::Client::connect(c, tcp.compat_write())
        .await
        .unwrap();
    client
        .simple_query(sql)
        .await
        .unwrap()
        .into_results()
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "needs Docker"]
async fn mssql_layers() {
    let node = MssqlServer::default()
        .with_accept_eula()
        .start()
        .await
        .unwrap();
    let port = node.get_host_port_ipv4(1433).await.unwrap();
    mssql_admin(
        port,
        "CREATE TABLE users (id int PRIMARY KEY, name nvarchar(50), password_hash nvarchar(50), \
           balance decimal(10,2), created datetime2 DEFAULT '2026-01-02T03:04:05'); \
         INSERT INTO users (id, name, password_hash, balance) VALUES (1, N'ada', N'h1', 12.50), (2, N'bob', N'h2', 0); \
         CREATE LOGIN ro WITH PASSWORD = 'Ro_pass_123!'; CREATE USER ro FOR LOGIN ro; \
         ALTER ROLE db_datareader ADD MEMBER ro;",
    )
    .await;
    let sa = format!(
        "mssql://sa:{}@127.0.0.1:{port}/master?trust_server_certificate=true",
        urlencode(MssqlServer::DEFAULT_SA_PASSWORD)
    );
    let (exec, conn) = setup(
        "mssql",
        &sa,
        json!({ "column_masks": ["users.password_hash"] }),
    );

    let spec = query(
        "SELECT id, name, password_hash, balance, created FROM users WHERE id = :id",
        json!([{ "name": "id", "schema": { "type": "integer" }, "required": true }]),
    );
    let out = exec
        .run_query(&conn, &spec, &args(json!({ "id": 1 })))
        .await
        .unwrap();
    assert_eq!(
        out.sql,
        "SELECT id, name, password_hash, balance, created FROM users WHERE id = @P1"
    );
    assert_eq!(
        out.rows,
        [[
            json!(1),
            json!("ada"),
            json!(MASKED),
            json!("12.50"),
            json!("2026-01-02T03:04:05")
        ]]
    );

    // Guard: rejected before reaching the server.
    for sql in [
        "DELETE FROM users",
        "EXEC xp_cmdshell 'dir'",
        "SELECT * FROM OPENROWSET('a', 'b', 'c')",
    ] {
        let e = exec
            .run_query(&conn, &query(sql, json!([])), &args(json!({})))
            .await
            .unwrap_err();
        assert!(matches!(e, SqlError::Rejected(_)), "{sql}: {e}");
    }

    // Session layer alone: a write is rolled back, so nothing changes.
    let pool = exec.pool_for(&conn).await.unwrap();
    let _ = run_read_only(&pool, "DELETE FROM users", &[], LIMITS).await;
    let n = run_read_only(&pool, "SELECT COUNT(*) FROM users", &[], LIMITS)
        .await
        .unwrap();
    assert_eq!(n.rows, [[json!(2)]], "the read session never keeps changes");

    // Row cap.
    let r = run_read_only(
        &pool,
        "SELECT id FROM users ORDER BY id",
        &[],
        Limits {
            row_limit: 1,
            timeout_ms: 5000,
        },
    )
    .await
    .unwrap();
    assert_eq!(r.rows.len(), 1);
    assert!(r.truncated);

    // Exploration and the probe.
    let tables = exec
        .run_query(&conn, &mode("list_tables"), &args(json!({})))
        .await
        .unwrap();
    assert!(
        tables.rows.iter().any(|r| r[1] == json!("users")),
        "{:?}",
        tables.rows
    );
    let cols = exec
        .run_query(
            &conn,
            &mode("describe_table"),
            &args(json!({ "table": "users" })),
        )
        .await
        .unwrap();
    assert_eq!(cols.rows[0][1], json!("id"));
    let report = exec.test_connection(&conn).await;
    assert!(report.ok, "{report:?}");
    assert_eq!(report.can_write, Some(true), "sa is sysadmin");
    let (_, ro) = setup(
        "mssql",
        &format!("mssql://ro:Ro_pass_123!@127.0.0.1:{port}/master?trust_server_certificate=true"),
        json!({}),
    );
    let report = exec.test_connection(&ro).await;
    assert_eq!(report.can_write, Some(false), "{report:?}");

    // Saved writes: capped and dry-runnable.
    let (exec, w) = setup(
        "mssql",
        &sa,
        json!({ "access_mode": "curated_write", "max_affected_rows": 1 }),
    );
    let upd = query(
        "UPDATE users SET name = :name WHERE id >= :id",
        json!([{ "name": "name", "schema": { "type": "string" } }, { "name": "id", "schema": { "type": "integer" } }]),
    );
    let e = exec
        .run_query(&w, &upd, &args(json!({ "name": "x", "id": 1 })))
        .await
        .unwrap_err();
    assert!(
        matches!(e, SqlError::TooManyRows { affected: 2, .. }),
        "{e}"
    );
    let dry = exec
        .run_query(
            &w,
            &upd,
            &args(json!({ "name": "x", "id": 2, "dry_run": true })),
        )
        .await
        .unwrap();
    assert_eq!(dry.write.unwrap().affected_rows, 1);
    let done = exec
        .run_query(&w, &upd, &args(json!({ "name": "zed", "id": 2 })))
        .await
        .unwrap();
    assert!(done.write.unwrap().committed);
    let names = run_read_only(&pool, "SELECT name FROM users ORDER BY id", &[], LIMITS)
        .await
        .unwrap();
    assert_eq!(names.rows, [[json!("ada")], [json!("zed")]]);
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

async fn clickhouse_admin(port: u16, sql: &str) {
    let r = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/"))
        .body(sql.to_owned())
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "{}", r.text().await.unwrap());
}

#[tokio::test]
#[ignore = "needs Docker"]
async fn clickhouse_layers() {
    let node = ClickHouse::default().start().await.unwrap();
    let port = node.get_host_port_ipv4(8123).await.unwrap();
    clickhouse_admin(
        port,
        "CREATE TABLE users (id UInt64, name String, password_hash String, score Nullable(Float64)) \
         ENGINE = MergeTree ORDER BY id",
    )
    .await;
    clickhouse_admin(
        port,
        "INSERT INTO users VALUES (1, 'ada', 'h1', 1.5), (2, 'bob', 'h2', NULL)",
    )
    .await;
    let dsn = format!("http://127.0.0.1:{port}/default");
    let (exec, conn) = setup(
        "clickhouse",
        &dsn,
        json!({ "column_masks": ["users.password_hash"] }),
    );

    let spec = query(
        "SELECT id, name, password_hash, score FROM users WHERE name = :name OR id = :id ORDER BY id",
        json!([
            { "name": "name", "schema": { "type": "string" } },
            { "name": "id", "schema": { "type": "integer" } }
        ]),
    );
    let out = exec
        .run_query(&conn, &spec, &args(json!({ "name": "ada", "id": 2 })))
        .await
        .unwrap();
    assert_eq!(
        out.rows,
        [
            [json!(1), json!("ada"), json!(MASKED), json!(1.5)],
            [json!(2), json!("bob"), json!(MASKED), json!(null)]
        ]
    );
    // A missing optional param is sent as NULL.
    let out = exec
        .run_query(&conn, &spec, &args(json!({ "name": "bob" })))
        .await
        .unwrap();
    assert_eq!(out.rows.len(), 1);

    for sql in [
        "INSERT INTO users VALUES (3, 'x', 'y', 0)",
        "SELECT * FROM url('http://example.com/a.csv', CSV)",
        "SELECT * FROM system.users",
    ] {
        let e = exec
            .run_query(&conn, &mode("run_sql"), &args(json!({ "sql": sql })))
            .await
            .unwrap_err();
        assert!(matches!(e, SqlError::Rejected(_)), "{sql}: {e}");
    }

    // Session layer alone: readonly=2 refuses writes.
    let pool = exec.pool_for(&conn).await.unwrap();
    let e = run_read_only(
        &pool,
        "INSERT INTO users VALUES (3, 'x', 'y', 0)",
        &[],
        LIMITS,
    )
    .await
    .unwrap_err();
    assert!(matches!(e, SqlError::Database(_)), "{e}");
    let r = run_read_only(&pool, "SELECT number FROM numbers(100)", &[], LIMITS)
        .await
        .unwrap();
    assert_eq!(r.rows.len(), 10);
    assert!(r.truncated);
    let e = run_read_only(
        &pool,
        "SELECT sleepEachRow(1) FROM numbers(10)",
        &[],
        Limits {
            row_limit: 10,
            timeout_ms: 1000,
        },
    )
    .await
    .unwrap_err();
    assert!(
        matches!(e, SqlError::Timeout(_) | SqlError::Database(_)),
        "{e}"
    );

    let tables = exec
        .run_query(&conn, &mode("list_tables"), &args(json!({})))
        .await
        .unwrap();
    assert_eq!(
        tables.rows,
        [[json!("default"), json!("users"), json!("table")]]
    );
    let cols = exec
        .run_query(
            &conn,
            &mode("describe_table"),
            &args(json!({ "table": "users" })),
        )
        .await
        .unwrap();
    assert_eq!(cols.rows.len(), 4);
    assert_eq!(cols.rows[3][3], json!(true), "score is nullable");
    let report = exec.test_connection(&conn).await;
    assert!(report.ok, "{report:?}");
    assert!(report.server_version.unwrap().starts_with("ClickHouse"));
}
