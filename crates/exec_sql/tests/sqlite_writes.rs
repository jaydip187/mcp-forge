//! SQLite writes: access-mode gating, WHERE enforcement, the affected-rows
//! cap, dry runs and the audit records they produce.

mod common;

use std::str::FromStr;

use common::{args, mode, query, setup};
use forge_core::AuditOutcome;
use forge_exec_sql::{tool_input, SqlError, StatementClass};
use serde_json::{json, Value};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, Connection, Executor};

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
        "CREATE TABLE orders (id INTEGER PRIMARY KEY, status TEXT NOT NULL, note TEXT);
         INSERT INTO orders VALUES (1, 'pending', NULL), (2, 'pending', NULL), (3, 'pending', NULL),
                                   (4, 'shipped', NULL), (5, 'shipped', NULL);",
    )
    .await
    .unwrap();
    c.close().await.unwrap();
    (dir, path.display().to_string())
}

async fn statuses(path: &str) -> Vec<String> {
    let mut c = SqliteConnectOptions::from_str(&format!("sqlite://{path}"))
        .unwrap()
        .connect()
        .await
        .unwrap();
    sqlx::query_scalar("SELECT status FROM orders ORDER BY id")
        .fetch_all(&mut c)
        .await
        .unwrap()
}

fn set_status() -> forge_core::SqlSpec {
    query(
        "UPDATE orders SET status = :status WHERE id = :id",
        json!([
            { "name": "id", "schema": { "type": "integer" }, "required": true },
            { "name": "status", "schema": { "type": "string", "enum": ["pending", "shipped", "cancelled"] }, "required": true }
        ]),
    )
}

#[tokio::test]
async fn curated_write_commits_and_is_audited() {
    let (_dir, path) = fixture().await;
    let (exec, conn) = setup(
        "sqlite",
        &path,
        json!({ "access_mode": "curated_write", "max_affected_rows": 2 }),
    );
    let spec = set_status();
    // Write tools gain `dry_run`; reads don't.
    let input = tool_input(&spec, &conn);
    assert!(input.schema["properties"]["dry_run"].is_object());
    let read = query("SELECT * FROM orders", json!([]));
    assert!(tool_input(&read, &conn).schema["properties"]
        .get("dry_run")
        .is_none());

    let run = exec
        .run(&conn, &spec, &args(json!({ "id": 1, "status": "shipped" })))
        .await;
    let out = run.result.unwrap();
    let w = out.write.as_ref().unwrap();
    assert_eq!((w.affected_rows, w.committed, w.dry_run), (1, true, false));
    assert_eq!(out.result_json()["affected_rows"], 1);
    let audit = run.audit.unwrap();
    assert_eq!(audit.outcome, AuditOutcome::Committed);
    assert_eq!(audit.class, StatementClass::Write);
    assert_eq!(audit.rows_affected, Some(1));
    assert_eq!(audit.sql, "UPDATE orders SET status = ? WHERE id = ?");
    assert_eq!(audit.params, [json!("shipped"), json!(1)]);
    assert_eq!(statuses(&path).await[0], "shipped");

    // Enum params are checked before anything runs.
    let run = exec
        .run(&conn, &spec, &args(json!({ "id": 2, "status": "lost" })))
        .await;
    assert!(matches!(run.result, Err(SqlError::InvalidArgs(_))));
    assert!(run.audit.is_none());
}

#[tokio::test]
async fn dry_run_reports_and_rolls_back() {
    let (_dir, path) = fixture().await;
    let (exec, conn) = setup("sqlite", &path, json!({ "access_mode": "curated_write" }));
    let spec = query(
        "UPDATE orders SET status = 'cancelled' WHERE status = :s",
        json!([{ "name": "s", "required": true }]),
    );
    let run = exec
        .run(
            &conn,
            &spec,
            &args(json!({ "s": "pending", "dry_run": true })),
        )
        .await;
    let w = run.result.unwrap().write.unwrap();
    assert_eq!((w.affected_rows, w.committed, w.dry_run), (3, false, true));
    assert_eq!(run.audit.unwrap().outcome, AuditOutcome::DryRun);
    assert_eq!(
        &statuses(&path).await[..3],
        ["pending", "pending", "pending"]
    );
}

#[tokio::test]
async fn cap_rolls_back_with_and_without_returning() {
    let (_dir, path) = fixture().await;
    let (exec, conn) = setup(
        "sqlite",
        &path,
        json!({ "access_mode": "curated_write", "max_affected_rows": 2 }),
    );
    let before = statuses(&path).await;
    let plain = query(
        "UPDATE orders SET status = 'cancelled' WHERE status = 'pending'",
        json!([]),
    );
    let run = exec.run(&conn, &plain, &args(json!({}))).await;
    assert_eq!(
        run.result.unwrap_err(),
        SqlError::TooManyRows {
            affected: 3,
            max: 2,
            at_least: false
        }
    );
    let audit = run.audit.unwrap();
    assert_eq!(audit.outcome, AuditOutcome::RolledBack);
    assert_eq!(audit.rows_affected, Some(3));
    assert!(audit.error.unwrap().contains("nothing was changed"));

    let returning = query("DELETE FROM orders WHERE id > 0 RETURNING id", json!([]));
    let e = exec
        .run(&conn, &returning, &args(json!({})))
        .await
        .result
        .unwrap_err();
    assert!(
        matches!(e, SqlError::TooManyRows { at_least: true, .. }),
        "{e:?}"
    );
    assert_eq!(statuses(&path).await, before, "nothing changed");

    // Within the cap, RETURNING rows come back.
    let insert = query(
        "INSERT INTO orders (status) VALUES ('pending') RETURNING id, status",
        json!([]),
    );
    let out = exec
        .run(&conn, &insert, &args(json!({})))
        .await
        .result
        .unwrap();
    assert_eq!(out.columns, ["id", "status"]);
    assert_eq!(out.rows, [[json!(6), json!("pending")]]);
    assert_eq!(out.write.unwrap().affected_rows, 1);
}

#[tokio::test]
async fn where_is_required_and_modes_gate_statements() {
    let (_dir, path) = fixture().await;
    let curated = |extra: Value| {
        let mut base = json!({ "access_mode": "curated_write" });
        for (k, v) in extra.as_object().unwrap() {
            base[k] = v.clone();
        }
        setup("sqlite", &path, base)
    };

    let (exec, conn) = curated(json!({}));
    for sql in ["DELETE FROM orders", "UPDATE orders SET note = 'x'"] {
        let run = exec
            .run(&conn, &query(sql, json!([])), &args(json!({})))
            .await;
        let e = run.result.unwrap_err();
        assert!(
            matches!(&e, SqlError::Rejected(m) if m.contains("WHERE")),
            "{sql}: {e:?}"
        );
        assert!(
            run.audit.is_none(),
            "rejected statements never reach the database"
        );
    }

    // run_sql only writes under full_access.
    let e = exec
        .run_query(
            &conn,
            &mode("run_sql"),
            &args(json!({ "sql": "UPDATE orders SET note = 'x' WHERE id = 1" })),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&e, SqlError::Rejected(m) if m.contains("saved tools")),
        "{e:?}"
    );
    // And read-only connections refuse curated writes.
    let (ro_exec, ro_conn) = setup("sqlite", &path, json!({}));
    let e = ro_exec
        .run_query(
            &ro_conn,
            &set_status(),
            &args(json!({ "id": 1, "status": "shipped" })),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&e, SqlError::Rejected(m) if m.contains("read-only")),
        "{e:?}"
    );

    let (fx, full) = curated(json!({ "access_mode": "full_access" }));
    let run = fx
        .run(
            &full,
            &mode("run_sql"),
            &args(json!({ "sql": "UPDATE orders SET note = 'x' WHERE id = 1" })),
        )
        .await;
    assert_eq!(run.result.unwrap().write.unwrap().affected_rows, 1);
    assert_eq!(run.audit.unwrap().outcome, AuditOutcome::Committed);
    for (sql, why) in [
        ("CREATE TABLE extra (a)", "schema changes"),
        ("PRAGMA writable_schema = 1", "never allowed"),
        ("ATTACH DATABASE 'x.db' AS x", "never allowed"),
    ] {
        let e = fx
            .run_query(&full, &mode("run_sql"), &args(json!({ "sql": sql })))
            .await
            .unwrap_err();
        assert!(
            matches!(&e, SqlError::Rejected(m) if m.contains(why)),
            "{sql}: {e:?}"
        );
    }
    let (dx, ddl) = curated(json!({ "access_mode": "full_access", "allow_ddl": true }));
    let out = dx
        .run_query(
            &ddl,
            &mode("run_sql"),
            &args(json!({ "sql": "CREATE TABLE extra (a)" })),
        )
        .await
        .unwrap();
    assert_eq!(out.write.unwrap().class, "ddl");
    // Reads on a writable connection still go through the read-only pool.
    let e = dx
        .run_query(
            &ddl,
            &query("SELECT 1", json!([])),
            &args(json!({ "dry_run": true })),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(e, SqlError::InvalidArgs(_)),
        "reads take no dry_run"
    );
}
