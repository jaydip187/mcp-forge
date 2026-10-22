use super::*;
use forge_core::ColumnMask;

const ALL: [Driver; 3] = [Driver::Sqlite, Driver::Postgres, Driver::Mysql];

#[test]
fn placeholders_postgres_reuse_numbers() {
    let b = bind_placeholders(
            "SELECT * FROM t WHERE a = :id OR b = :id AND c::text = :name -- :not_me\n AND d = ':nor_me'",
            Driver::Postgres,
        )
        .unwrap();
    assert_eq!(
        b.sql,
        "SELECT * FROM t WHERE a = $1 OR b = $1 AND c::text = $2 -- :not_me\n AND d = ':nor_me'"
    );
    assert_eq!(b.binds, ["id", "name"]);
    assert_eq!(b.names, ["id", "name"]);
}

#[test]
fn placeholders_question_marks_per_occurrence() {
    for d in [Driver::Sqlite, Driver::Mysql] {
        let b = bind_placeholders("SELECT :a, :b, :a, 'Ã©' || :b", d).unwrap();
        assert_eq!(b.sql, "SELECT ?, ?, ?, 'Ã©' || ?");
        assert_eq!(b.binds, ["a", "b", "a", "b"]);
        assert_eq!(b.names, ["a", "b"]);
    }
    // Multi-line with non-ASCII before the placeholder.
    let b = bind_placeholders("SELECT 'Ã¼'\n  , x FROM t WHERE y = :y", Driver::Sqlite).unwrap();
    assert_eq!(b.sql, "SELECT 'Ã¼'\n  , x FROM t WHERE y = ?");
    assert!(bind_placeholders("SELECT ?", Driver::Sqlite).is_err());
    assert!(bind_placeholders("SELECT $1", Driver::Postgres).is_err());
}

#[test]
fn accepts_reads() {
    for d in ALL {
        for sql in [
            "SELECT 1",
            "select id, name from users where id = ?",
            "WITH x AS (SELECT 1 AS a) SELECT a FROM x",
            "SELECT count(*) FROM orders o JOIN users u ON u.id = o.user_id",
            "SELECT * FROM (SELECT 1) AS s UNION ALL SELECT 2",
            "SELECT 1;",
        ] {
            let sql = if d == Driver::Postgres {
                sql.replace('?', "$1")
            } else {
                sql.to_owned()
            };
            check_read_only(&sql, d).unwrap_or_else(|e| panic!("{d:?} {sql}: {e}"));
        }
    }
    let q = check_read_only(
        "SELECT u.email FROM public.users u JOIN orders ON true",
        Driver::Postgres,
    )
    .unwrap();
    assert_eq!(q.tables, ["orders", "public.users"]);
}

#[test]
fn rejects_writes_and_multiple_statements() {
    for d in ALL {
        for sql in [
            "DELETE FROM users",
            "UPDATE users SET a = 1",
            "INSERT INTO users VALUES (1)",
            "DROP TABLE users",
            "CREATE TABLE x (a int)",
            "SELECT 1; DELETE FROM users",
            "SELECT 1; SELECT 2",
            "",
            "EXPLAIN ANALYZE DELETE FROM users",
        ] {
            assert!(check_read_only(sql, d).is_err(), "{d:?} accepted {sql}");
        }
    }
    for sql in [
        "SELECT * FROM users FOR UPDATE",
        "SELECT * INTO new_users FROM users",
        "WITH gone AS (DELETE FROM users RETURNING *) SELECT * FROM gone",
        "WITH x AS (UPDATE users SET a = 1 RETURNING id) SELECT count(*) FROM x",
        "WITH x AS (INSERT INTO t VALUES (1) RETURNING *) SELECT * FROM x",
        "SELECT * FROM (SELECT 1) s WHERE EXISTS (SELECT 1 FROM t FOR SHARE)",
        "COPY users TO '/tmp/x'",
        "SET statement_timeout = 0",
        "SET TRANSACTION READ WRITE",
        "VACUUM",
    ] {
        let e = check_read_only(sql, Driver::Postgres).unwrap_err();
        assert!(!e.is_empty(), "{sql}");
    }
    let e = check_read_only(
        "WITH gone AS (DELETE FROM users RETURNING *) SELECT * FROM gone",
        Driver::Postgres,
    )
    .unwrap_err();
    assert!(e.contains("DELETE"), "{e}");
}

#[test]
fn classifies_statements() {
    use StatementClass::*;
    let c = |sql: &str, d| check_statement(sql, d).map(|q| q.class);
    for d in ALL {
        assert_eq!(c("SELECT 1", d), Ok(Read));
        assert_eq!(c("INSERT INTO t (a) VALUES (1)", d), Ok(Write));
        assert_eq!(c("UPDATE t SET a = 1 WHERE id = 2", d), Ok(Write));
        assert_eq!(c("DELETE FROM t WHERE id = 2", d), Ok(Write));
        assert_eq!(c("CREATE TABLE x (a int)", d), Ok(Ddl));
        assert_eq!(c("DROP TABLE x", d), Ok(Ddl));
        assert_eq!(c("ALTER TABLE x ADD COLUMN b int", d), Ok(Ddl));
    }
    assert_eq!(c("TRUNCATE t", Driver::Postgres), Ok(Ddl));
    assert_eq!(c("GRANT SELECT ON t TO bob", Driver::Postgres), Ok(Admin));
    assert_eq!(c("SET search_path = evil", Driver::Postgres), Ok(Admin));
    assert_eq!(c("COPY t TO '/tmp/x'", Driver::Postgres), Ok(Admin));
    assert_eq!(c("PRAGMA writable_schema = 1", Driver::Sqlite), Ok(Admin));
    assert_eq!(c("SELECT * FROM t FOR UPDATE", Driver::Postgres), Ok(Write));
    assert_eq!(c("SELECT * INTO y FROM t", Driver::Postgres), Ok(Ddl));
    let q = check_statement(
        "UPDATE t SET a = 1 WHERE id = 2 RETURNING id",
        Driver::Postgres,
    )
    .unwrap();
    assert!(q.returning);
    assert_eq!(q.tables, ["t"]);
    let q = check_statement("INSERT INTO s.t (a) SELECT a FROM u", Driver::Postgres).unwrap();
    assert_eq!(q.tables, ["s.t", "u"]);
    assert!(!q.returning);
}

#[test]
fn writes_need_where_and_cant_hide_in_queries() {
    for d in ALL {
        for sql in ["UPDATE t SET a = 1", "DELETE FROM t"] {
            let e = check_statement(sql, d).unwrap().missing_where.unwrap();
            assert!(e.contains("WHERE"), "{d:?} {sql}: {e}");
        }
        assert!(check_statement("DELETE FROM t WHERE true", d)
            .unwrap()
            .missing_where
            .is_none());
    }
    for sql in [
        "WITH gone AS (DELETE FROM t WHERE true RETURNING *) SELECT count(*) FROM gone",
        "WITH x AS (UPDATE t SET a = 1 WHERE true RETURNING id) INSERT INTO log SELECT id FROM x",
    ] {
        let e = check_statement(sql, Driver::Postgres).unwrap_err();
        assert!(e.contains("inside the query"), "{sql}: {e}");
    }
}

#[test]
fn cte_references_are_not_tables_but_shadowing_is_caught() {
    let t = |sql: &str| check_read_only(sql, Driver::Postgres).unwrap().tables;
    assert_eq!(
        t("WITH x AS (SELECT * FROM orders) SELECT * FROM x"),
        ["orders"]
    );
    // Non-recursive: inside its own body, `secrets` is the real table.
    assert_eq!(
        t("WITH secrets AS (SELECT * FROM secrets) SELECT * FROM secrets"),
        ["secrets"]
    );
    // A later sibling isn't visible to an earlier one.
    assert_eq!(
        t("WITH a AS (SELECT * FROM b), b AS (SELECT 1) SELECT * FROM a"),
        ["b"]
    );
    // A CTE inside a subquery doesn't hide the outer real table.
    assert_eq!(
        t("SELECT * FROM (WITH users AS (SELECT 1) SELECT * FROM users) s JOIN users ON true"),
        ["users"]
    );
    assert!(t("WITH RECURSIVE c(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM c WHERE n < 3) SELECT * FROM c").is_empty());
    // A bare `TABLE secrets` statement doesn't parse (so it's refused).
    assert!(check_read_only("TABLE secrets", Driver::Postgres).is_err());
    assert_eq!(t("SELECT 1 UNION TABLE public.s"), ["public.s"]);
}

#[test]
fn allowlist_and_system_tables() {
    let allow = |list: &[&str], t: &str, d| {
        table_allowed(
            t,
            &list.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            d,
        )
    };
    assert!(allow(&[], "anything", Driver::Postgres));
    assert!(allow(&["users"], "USERS", Driver::Postgres));
    assert!(allow(&["users"], "sales.users", Driver::Postgres));
    assert!(allow(&["public.users"], "users", Driver::Postgres));
    assert!(!allow(&["sales.users"], "users", Driver::Postgres));
    assert!(allow(&["sales.*"], "sales.orders", Driver::Postgres));
    assert!(!allow(&["sales.*"], "orders", Driver::Postgres));
    assert!(allow(&["main.*"], "orders", Driver::Sqlite));
    assert!(!allow(&["shop.users"], "users", Driver::Mysql));
    assert!(allow(&["*"], "x.y", Driver::Mysql));
    assert!(!allow(&["users"], "orders", Driver::Mysql));

    for (t, d) in [
        ("pg_catalog.pg_authid", Driver::Postgres),
        ("pg_user", Driver::Postgres),
        ("information_schema.tables", Driver::Mysql),
        ("mysql.user", Driver::Mysql),
        ("sqlite_master", Driver::Sqlite),
        ("main.sqlite_schema", Driver::Sqlite),
    ] {
        assert!(is_system_table(t, d), "{t}");
    }
    assert!(!is_system_table("pg_orders", Driver::Mysql));
    assert!(!is_system_table("public.users", Driver::Postgres));
}

#[test]
fn masks_block_every_way_to_read_the_column() {
    let masks = [ColumnMask::parse("users.password_hash").unwrap()];
    let check = |sql: &str| {
        let q = check_read_only(sql, Driver::Postgres).unwrap();
        check_masks(&q, &masks)
    };
    check("SELECT * FROM users").unwrap();
    check("SELECT u.* FROM users u").unwrap();
    check("SELECT id, name FROM users WHERE id = 1").unwrap();
    check("SELECT count(*) FROM users").unwrap();
    check("SELECT password_hash FROM orders").unwrap(); // mask is for users only
    for sql in [
        "SELECT password_hash FROM users",
        "SELECT password_hash AS p FROM users",
        "SELECT id FROM users WHERE password_hash LIKE 'a%'",
        "SELECT md5(u.password_hash) FROM users u",
        "SELECT to_json(u) FROM users u",
        "SELECT row_to_json(x) FROM (SELECT id FROM users) x",
        "SELECT json_agg(u.*) FROM users u",
        "SELECT * FROM (SELECT * FROM users) x",
        "SELECT 'a' AS a, 'b' AS b, 'c' AS c, 'd' AS d UNION ALL SELECT * FROM users",
        "WITH x AS (SELECT * FROM users) SELECT id FROM x",
    ] {
        assert!(check(sql).is_err(), "{sql}");
    }
    let any = [ColumnMask::parse("*.email").unwrap()];
    let q = check_read_only("SELECT email FROM anything", Driver::Sqlite).unwrap();
    assert!(check_masks(&q, &any).is_err());
}

#[test]
fn rejects_denied_functions_anywhere() {
    for (d, sql) in [
        (Driver::Postgres, "SELECT pg_terminate_backend(123)"),
        (
            Driver::Postgres,
            "SELECT pg_catalog.pg_read_file('/etc/passwd')",
        ),
        (
            Driver::Postgres,
            "SELECT * FROM dblink('host=x', 'DELETE FROM t') AS t(a int)",
        ),
        (Driver::Postgres, "SELECT lo_import('/etc/passwd')"),
        (Driver::Postgres, "SELECT NEXTVAL('seq')"),
        (Driver::Postgres, "SELECT setval('seq', 1)"),
        (
            Driver::Postgres,
            "SELECT a FROM t WHERE a IN (SELECT pg_advisory_lock(1))",
        ),
        (
            Driver::Postgres,
            "SELECT query_to_xml('DELETE FROM t', true, true, '')",
        ),
        (Driver::Mysql, "SELECT SLEEP(10)"),
        (
            Driver::Postgres,
            "SELECT table_to_xml('secrets', true, true, '')",
        ),
        (
            Driver::Postgres,
            "SELECT * FROM ts_stat('SELECT body FROM secrets')",
        ),
        (Driver::Mysql, "SELECT LOAD_FILE('/etc/passwd')"),
        (Driver::Mysql, "SELECT BENCHMARK(1000000, MD5('x'))"),
        (Driver::Sqlite, "SELECT load_extension('evil')"),
        (Driver::Sqlite, "SELECT writefile('x', 'y')"),
    ] {
        let e = check_read_only(sql, d).unwrap_err();
        assert!(e.contains("not allowed"), "{sql}: {e}");
    }
    // Lookalikes are fine.
    check_read_only("SELECT lower(name), sleepy FROM t", Driver::Mysql).unwrap();
}
