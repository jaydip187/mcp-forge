//! Postgres and MySQL in Docker (testcontainers): curated queries work, and
//! writes are refused at each layer. Run with `cargo test -p forge-exec-sql
//! --test containers -- --ignored` (needs a running Docker daemon).

mod common;

use std::time::Instant;

use common::{args, mode, query, setup};
use forge_exec_sql::{run_read_only, Limits, SqlError, MASKED};
use serde_json::json;
use sqlx::{Connection, Executor};
use testcontainers_modules::mysql::Mysql;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::ImageExt;

const LIMITS: Limits = Limits {
    row_limit: 10,
    timeout_ms: 2000,
};

#[tokio::test]
#[ignore = "needs Docker"]
async fn postgres_layers() {
    let node = Postgres::default()
        .with_tag("16-alpine")
        .start()
        .await
        .unwrap();
    let port = node.get_host_port_ipv4(5432).await.unwrap();
    let admin_dsn = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let mut admin = sqlx::PgConnection::connect(&admin_dsn).await.unwrap();
    admin
        .execute(
            "CREATE TABLE users (id int PRIMARY KEY, name text, email text, password_hash text, \
               balance numeric(10,2), meta jsonb, created timestamptz DEFAULT '2026-01-02T03:04:05Z');
             INSERT INTO users (id, name, email, password_hash, balance, meta) VALUES
               (1, 'ada', 'ada@x.io', 'h1', 12.50, '{\"vip\": true}'), (2, 'bob', NULL, 'h2', 0, NULL);
             CREATE SEQUENCE s;
             CREATE ROLE ro LOGIN PASSWORD 'ro_pass_123';
             GRANT SELECT ON users TO ro;",
        )
        .await
        .unwrap();

    let (exec, conn) = setup(
        "postgres",
        &admin_dsn,
        json!({ "column_masks": ["users.password_hash"] }),
    );

    // Curated query: typed bind, masks, value conversion.
    let spec = query(
        "SELECT id, name, password_hash, balance, meta, created FROM users WHERE id = :id",
        json!([{ "name": "id", "schema": { "type": "integer" }, "required": true }]),
    );
    let out = exec
        .run_query(&conn, &spec, &args(json!({ "id": 1 })))
        .await
        .unwrap();
    assert_eq!(
        out.sql,
        "SELECT id, name, password_hash, balance, meta, created FROM users WHERE id = $1"
    );
    assert_eq!(
        out.rows,
        [[
            json!(1),
            json!("ada"),
            json!(MASKED),
            json!("12.50"),
            json!({ "vip": true }),
            json!("2026-01-02T03:04:05+00:00")
        ]]
    );

    // Layers 3–4: rejected before reaching the database.
    for sql in [
        "DELETE FROM users",
        "WITH gone AS (DELETE FROM users RETURNING *) SELECT * FROM gone",
        "SELECT * FROM users FOR UPDATE",
        "SELECT nextval('s')",
        "SELECT pg_sleep(5)",
        "SELECT pg_terminate_backend(1)",
        "SELECT 1; DELETE FROM users",
    ] {
        let e = exec
            .run_query(&conn, &query(sql, json!([])), &args(json!({})))
            .await
            .unwrap_err();
        assert!(matches!(e, SqlError::Rejected(_)), "{sql}: {e:?}");
    }

    // Layer 2 alone: the read-only transaction refuses writes, even as superuser.
    let pool = exec.pool_for(&conn).await.unwrap();
    for sql in [
        "DELETE FROM users",
        "WITH gone AS (DELETE FROM users RETURNING *) SELECT * FROM gone",
        "SELECT nextval('s')",
        "CREATE TABLE t (a int)",
        "UPDATE users SET name = 'x'",
    ] {
        let e = run_read_only(&pool, sql, &[], LIMITS).await.unwrap_err();
        assert!(
            matches!(&e, SqlError::Database(m) if m.contains("read-only transaction")),
            "{sql}: {e:?}"
        );
    }
    // statement_timeout stops long queries server-side.
    let started = Instant::now();
    let e = run_read_only(
        &pool,
        "SELECT pg_sleep(10)",
        &[],
        Limits {
            row_limit: 1,
            timeout_ms: 500,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(e, SqlError::Timeout(_)), "{e:?}");
    assert!(started.elapsed().as_secs() < 2);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&mut admin)
        .await
        .unwrap();
    assert_eq!(n, 2, "nothing was deleted");

    // Layer 1: the privilege check tells superusers from SELECT-only roles.
    let r = exec.test_connection(&conn).await;
    assert!(r.ok, "{r:?}");
    assert_eq!(r.can_write, Some(true));
    assert!(r.write_privileges.iter().any(|p| p == "superuser"), "{r:?}");
    assert!(r
        .server_version
        .as_deref()
        .unwrap()
        .starts_with("PostgreSQL 16"));

    let ro_dsn = format!("postgres://ro:ro_pass_123@127.0.0.1:{port}/postgres");
    let (ro_exec, ro_conn) = setup("postgres", &ro_dsn, json!({}));
    let r = ro_exec.test_connection(&ro_conn).await;
    assert!(r.ok, "{r:?}");
    assert_eq!(r.user.as_deref(), Some("ro"));
    assert_eq!(r.can_write, Some(false), "{r:?}");
    assert!(r.warnings.is_empty());

    // Exploration tools: catalog SQL works, allowlist and masks apply.
    admin
        .execute("CREATE SCHEMA sales; CREATE TABLE sales.users (id int PRIMARY KEY);")
        .await
        .unwrap();
    let (exp, econn) = setup(
        "postgres",
        &admin_dsn,
        json!({ "table_allowlist": ["public.users"], "column_masks": ["users.password_hash"] }),
    );
    let out = exp
        .run_query(&econn, &mode("list_tables"), &args(json!({})))
        .await
        .unwrap();
    assert_eq!(
        out.rows,
        [[json!("public"), json!("users"), json!("table")]]
    );
    let out = exp
        .run_query(
            &econn,
            &mode("describe_table"),
            &args(json!({ "table": "users" })),
        )
        .await
        .unwrap();
    assert_eq!(
        out.rows[0],
        [
            json!("id"),
            json!("integer"),
            json!(false),
            json!(null),
            json!(true),
            json!(false)
        ]
    );
    let hash = out.rows.iter().find(|r| r[0] == "password_hash").unwrap();
    assert_eq!(hash[5], json!(true));
    let e = exp
        .run_query(
            &econn,
            &mode("describe_table"),
            &args(json!({ "table": "sales.users" })),
        )
        .await
        .unwrap_err();
    assert!(matches!(e, SqlError::Rejected(_)), "{e:?}");
    let out = exp
        .run_query(
            &econn,
            &mode("run_sql"),
            &args(json!({ "sql": "SELECT id, name FROM users ORDER BY id" })),
        )
        .await
        .unwrap();
    assert_eq!(
        out.rows,
        [[json!(1), json!("ada")], [json!(2), json!("bob")]]
    );
    for sql in [
        "SELECT * FROM pg_user",
        "SELECT rolname FROM pg_catalog.pg_roles",
        "SELECT * FROM sales.users",
        "SELECT to_json(u) FROM users u",
        "SELECT table_to_xml('users', true, true, '')",
    ] {
        let e = exp
            .run_query(&econn, &mode("run_sql"), &args(json!({ "sql": sql })))
            .await
            .unwrap_err();
        assert!(matches!(e, SqlError::Rejected(_)), "{sql}: {e:?}");
    }

    // Wrong password: the error never contains the DSN or password.
    let bad = format!("postgres://ro:wrong_secret_pw@127.0.0.1:{port}/postgres");
    let (bad_exec, bad_conn) = setup("postgres", &bad, json!({}));
    let r = bad_exec.test_connection(&bad_conn).await;
    let err = r.error.unwrap();
    assert!(!err.contains("wrong_secret_pw"), "{err}");
}

#[tokio::test]
#[ignore = "needs Docker"]
async fn mysql_layers() {
    let node = Mysql::default().with_tag("8.4").start().await.unwrap();
    let port = node.get_host_port_ipv4(3306).await.unwrap();
    let admin_dsn = format!("mysql://root@127.0.0.1:{port}/test");
    let mut admin = sqlx::MySqlConnection::connect(&admin_dsn).await.unwrap();
    for stmt in [
        "CREATE TABLE users (id INT PRIMARY KEY, name VARCHAR(50), email VARCHAR(100), \
           balance DECIMAL(10,2), flags TINYINT(1), meta JSON, created DATETIME)",
        "INSERT INTO users VALUES (1, 'ada', 'ada@x.io', 12.50, 1, JSON_OBJECT('vip', true), '2026-01-02 03:04:05'), \
           (2, 'bob', NULL, 0, 0, NULL, NULL)",
        "CREATE USER 'ro'@'%' IDENTIFIED BY 'ro_pass_123'",
        "GRANT SELECT ON test.* TO 'ro'@'%'",
    ] {
        admin.execute(stmt).await.unwrap();
    }

    let (exec, conn) = setup("mysql", &admin_dsn, json!({ "column_masks": ["*.email"] }));
    let spec = query(
        "SELECT id, name, email, balance, flags, meta, created FROM users WHERE id = :id OR name = :id",
        json!([{ "name": "id", "required": true }]),
    );
    let out = exec
        .run_query(&conn, &spec, &args(json!({ "id": "1" })))
        .await
        .unwrap();
    assert_eq!(
        out.sql,
        "SELECT id, name, email, balance, flags, meta, created FROM users WHERE id = ? OR name = ?"
    );
    assert_eq!(out.params, [json!("1"), json!("1")]);
    assert_eq!(
        out.rows,
        [[
            json!(1),
            json!("ada"),
            json!(MASKED),
            json!("12.50"),
            json!(true),
            json!({ "vip": true }),
            json!("2026-01-02T03:04:05")
        ]]
    );

    for sql in [
        "DELETE FROM users",
        "UPDATE users SET name = 'x'",
        "SELECT SLEEP(5)",
        "SELECT LOAD_FILE('/etc/passwd')",
        "SELECT * FROM users FOR UPDATE",
        "SELECT 1; DELETE FROM users",
    ] {
        let e = exec
            .run_query(&conn, &query(sql, json!([])), &args(json!({})))
            .await
            .unwrap_err();
        assert!(matches!(e, SqlError::Rejected(_)), "{sql}: {e:?}");
    }

    let pool = exec.pool_for(&conn).await.unwrap();
    for sql in [
        "DELETE FROM users",
        "UPDATE users SET name = 'x'",
        "INSERT INTO users (id) VALUES (9)",
    ] {
        let e = run_read_only(&pool, sql, &[], LIMITS).await.unwrap_err();
        assert!(
            matches!(&e, SqlError::Database(m) if m.to_lowercase().contains("read only")),
            "{sql}: {e:?}"
        );
    }
    let started = Instant::now();
    let e = run_read_only(
        &pool,
        "SELECT COUNT(*) FROM information_schema.columns a, information_schema.columns b, \
         information_schema.columns c",
        &[],
        Limits {
            row_limit: 1,
            timeout_ms: 500,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(e, SqlError::Timeout(_)), "{e:?}");
    assert!(started.elapsed().as_secs() < 3);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&mut admin)
        .await
        .unwrap();
    assert_eq!(n, 2, "nothing was deleted");

    let r = exec.test_connection(&conn).await;
    assert!(r.ok, "{r:?}");
    assert_eq!(r.can_write, Some(true), "{r:?}");
    assert!(r
        .server_version
        .as_deref()
        .unwrap()
        .starts_with("MySQL 8.4"));

    let ro_dsn = format!("mysql://ro:ro_pass_123@127.0.0.1:{port}/test");
    let (ro_exec, ro_conn) = setup("mysql", &ro_dsn, json!({}));
    let r = ro_exec.test_connection(&ro_conn).await;
    assert!(r.ok, "{r:?}");
    assert_eq!(r.can_write, Some(false), "{r:?}");

    // Exploration tools on MySQL's catalog.
    let (exp, econn) = setup(
        "mysql",
        &admin_dsn,
        json!({ "table_allowlist": ["users"], "column_masks": ["*.email"] }),
    );
    let out = exp
        .run_query(&econn, &mode("list_tables"), &args(json!({})))
        .await
        .unwrap();
    assert_eq!(out.rows, [[json!("test"), json!("users"), json!("table")]]);
    let out = exp
        .run_query(
            &econn,
            &mode("describe_table"),
            &args(json!({ "table": "USERS" })),
        )
        .await
        .unwrap();
    assert_eq!(
        out.rows[0],
        [
            json!("id"),
            json!("int"),
            json!(false),
            json!(null),
            json!(true),
            json!(false)
        ]
    );
    assert_eq!(out.rows[2][0], json!("email"));
    assert_eq!(out.rows[2][5], json!(true));
    let out = exp
        .run_query(
            &econn,
            &mode("run_sql"),
            &args(json!({ "sql": "SELECT * FROM users ORDER BY id LIMIT 1" })),
        )
        .await
        .unwrap();
    assert_eq!(out.rows[0][2], json!(MASKED));
    for sql in [
        "SELECT user FROM mysql.user",
        "SELECT email FROM users",
        "SELECT * FROM information_schema.tables",
    ] {
        let e = exp
            .run_query(&econn, &mode("run_sql"), &args(json!({ "sql": sql })))
            .await
            .unwrap_err();
        assert!(matches!(e, SqlError::Rejected(_)), "{sql}: {e:?}");
    }
}
