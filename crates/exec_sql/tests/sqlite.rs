//! SQLite: curated queries work, and writes are refused at every layer.

mod common;

use std::str::FromStr;

use common::{args, query, setup};
use forge_exec_sql::{run_read_only, Limits, SqlError, MASKED};
use serde_json::json;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, Connection, Executor};

/// A file DB with `users` (3 rows), created through a separate writable connection.
async fn fixture() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shop.db");
    let mut c = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))
        .unwrap()
        .create_if_missing(true)
        .connect()
        .await
        .unwrap();
    c.execute(
        "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, email TEXT, password_hash TEXT, score REAL, avatar BLOB);
         INSERT INTO users VALUES (1, 'ada', 'ada@x.io', 'h1', 9.5, x'0102'), (2, 'bob', NULL, 'h2', NULL, NULL), (3, 'cy', 'cy@x.io', 'h3', 1.0, NULL);",
    )
    .await
    .unwrap();
    c.close().await.unwrap();
    (dir, path.display().to_string())
}

async fn user_count(path: &str) -> i64 {
    let mut c = SqliteConnectOptions::from_str(&format!("sqlite://{path}"))
        .unwrap()
        .connect()
        .await
        .unwrap();
    sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&mut c)
        .await
        .unwrap()
}

#[tokio::test]
async fn curated_query_binds_params_and_masks() {
    let (_dir, path) = fixture().await;
    let (exec, conn) = setup(
        "sqlite",
        &path,
        json!({ "column_masks": ["users.password_hash", "*.email"] }),
    );
    let spec = query(
        "SELECT id, name, email, password_hash, score, avatar FROM users WHERE id >= :min ORDER BY id",
        json!([{ "name": "min", "schema": { "type": "integer" }, "required": true }]),
    );
    let out = exec
        .run_query(&conn, &spec, &args(json!({ "min": 2 })))
        .await
        .unwrap();
    assert_eq!(
        out.sql,
        "SELECT id, name, email, password_hash, score, avatar FROM users WHERE id >= ? ORDER BY id"
    );
    assert_eq!(out.params, [json!(2)]);
    assert_eq!(
        out.result_json(),
        json!({
            "columns": ["id", "name", "email", "password_hash", "score", "avatar"],
            "rows": [
                [2, "bob", null, MASKED, null, null],
                [3, "cy", MASKED, MASKED, 1.0, null]
            ],
            "truncated": false
        })
    );
    assert_eq!(out.masked_columns, ["email", "password_hash"]);

    let out = exec
        .run_query(
            &conn,
            &query("SELECT avatar FROM users WHERE id = 1", json!([])),
            &args(json!({})),
        )
        .await
        .unwrap();
    assert_eq!(out.rows, [[json!("<binary, 2 bytes>")]]);
}

#[tokio::test]
async fn limits_rows_cells_and_reports_columns_when_empty() {
    let (_dir, path) = fixture().await;
    let (exec, conn) = setup("sqlite", &path, json!({ "row_limit": 2 }));
    let out = exec
        .run_query(
            &conn,
            &query("SELECT id FROM users ORDER BY id", json!([])),
            &args(json!({})),
        )
        .await
        .unwrap();
    assert_eq!(out.rows, [[json!(1)], [json!(2)]]);
    assert!(out.truncated);

    let out = exec
        .run_query(
            &conn,
            &query("SELECT id, name FROM users WHERE id = -1", json!([])),
            &args(json!({})),
        )
        .await
        .unwrap();
    assert_eq!(out.columns, ["id", "name"]);
    assert!(out.rows.is_empty() && !out.truncated);

    let out = exec
        .run_query(
            &conn,
            &query("SELECT printf('%.5000c', 'x') AS big", json!([])),
            &args(json!({})),
        )
        .await
        .unwrap();
    let cell = out.rows[0][0].as_str().unwrap();
    assert!(
        cell.ends_with("… [4000 more chars]"),
        "{}",
        &cell[cell.len() - 40..]
    );
}

#[tokio::test]
async fn args_are_bound_never_interpolated() {
    let (_dir, path) = fixture().await;
    let (exec, conn) = setup("sqlite", &path, json!({}));
    let spec = query(
        "SELECT id FROM users WHERE name = :name",
        json!([{ "name": "name", "required": true }]),
    );
    let out = exec
        .run_query(
            &conn,
            &spec,
            &args(json!({ "name": "x' OR '1'='1'; DELETE FROM users; --" })),
        )
        .await
        .unwrap();
    assert!(out.rows.is_empty());
    assert_eq!(user_count(&path).await, 3);

    // Bad args are reported per argument, before touching the database.
    let e = exec
        .run_query(&conn, &spec, &args(json!({ "name": 5, "extra": 1 })))
        .await
        .unwrap_err();
    let SqlError::InvalidArgs(v) = e else {
        panic!("{e:?}")
    };
    assert!(v.has("name") && v.has("extra"), "{v}");
}

#[tokio::test]
async fn layer3_ast_rejects_writes() {
    let (_dir, path) = fixture().await;
    let (exec, conn) = setup("sqlite", &path, json!({}));
    for sql in [
        "DELETE FROM users",
        "UPDATE users SET name = 'x'",
        "INSERT INTO users (id) VALUES (9)",
        "DROP TABLE users",
        "SELECT 1; DELETE FROM users",
        "PRAGMA query_only = OFF",
        "ATTACH DATABASE 'other.db' AS o",
    ] {
        let e = exec
            .run_query(&conn, &query(sql, json!([])), &args(json!({})))
            .await
            .unwrap_err();
        assert!(matches!(e, SqlError::Rejected(_)), "{sql}: {e:?}");
    }
    assert_eq!(user_count(&path).await, 3);
}

#[tokio::test]
async fn layer4_denylist_rejects_functions() {
    let (_dir, path) = fixture().await;
    let (exec, conn) = setup("sqlite", &path, json!({}));
    for sql in [
        "SELECT load_extension('evil')",
        "SELECT name FROM users WHERE id IN (SELECT writefile('x', 'y'))",
    ] {
        let e = exec
            .run_query(&conn, &query(sql, json!([])), &args(json!({})))
            .await
            .unwrap_err();
        assert!(
            matches!(&e, SqlError::Rejected(m) if m.contains("not allowed")),
            "{sql}: {e:?}"
        );
    }
}

#[tokio::test]
async fn layer2_session_refuses_writes_without_the_guard() {
    let (_dir, path) = fixture().await;
    let (exec, conn) = setup("sqlite", &path, json!({}));
    let pool = exec.pool_for(&conn).await.unwrap();
    let limits = Limits {
        row_limit: 10,
        timeout_ms: 2000,
    };
    // Both read-only mechanisms are active on the connection.
    let r = run_read_only(&pool, "PRAGMA query_only", &[], limits)
        .await
        .unwrap();
    assert_eq!(r.rows, [[json!(1)]]);
    for sql in [
        "DELETE FROM users",
        "INSERT INTO users (id) VALUES (9)",
        "CREATE TABLE t (a)",
        "UPDATE users SET name = 'x' RETURNING id",
    ] {
        let e = run_read_only(&pool, sql, &[], limits).await.unwrap_err();
        assert!(matches!(e, SqlError::Database(_)), "{sql}: {e:?}");
    }
    assert_eq!(user_count(&path).await, 3);
}

#[tokio::test]
async fn timeout_interrupts_runaway_queries() {
    let (_dir, path) = fixture().await;
    let (exec, conn) = setup("sqlite", &path, json!({ "timeout_ms": 300 }));
    let spec = query(
        "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT count(*) FROM c",
        json!([]),
    );
    let started = std::time::Instant::now();
    let e = exec
        .run_query(&conn, &spec, &args(json!({})))
        .await
        .unwrap_err();
    assert_eq!(e, SqlError::Timeout(300));
    assert!(started.elapsed().as_secs() < 2, "{:?}", started.elapsed());
    // The pooled connection is usable again afterwards.
    exec.run_query(&conn, &query("SELECT 1", json!([])), &args(json!({})))
        .await
        .unwrap();
}

#[tokio::test]
async fn test_connection_reports_version_and_write_access() {
    let (_dir, path) = fixture().await;
    let (exec, conn) = setup("sqlite", &path, json!({}));
    let r = exec.test_connection(&conn).await;
    assert!(r.ok, "{r:?}");
    assert!(r.server_version.as_deref().unwrap().starts_with("SQLite 3"));
    assert_eq!(r.can_write, Some(true), "temp files are writable");
    assert!(!r.warnings.is_empty());

    let (exec, conn) = setup("sqlite", &format!("{path}.missing"), json!({}));
    let r = exec.test_connection(&conn).await;
    assert!(!r.ok && r.error.is_some(), "{r:?}");
}
