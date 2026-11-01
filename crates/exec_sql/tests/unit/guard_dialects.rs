//! Stage 25: DuckDB, SQL Server and ClickHouse through the same guard.

use super::*;

const NEW: [Driver; 3] = [Driver::Duckdb, Driver::Mssql, Driver::Clickhouse];

#[test]
fn placeholders_per_driver() {
    let sql = "SELECT * FROM t WHERE a = :id OR b = :id AND c = :name";
    let b = bind_placeholders(sql, Driver::Mssql).unwrap();
    assert_eq!(
        b.sql,
        "SELECT * FROM t WHERE a = @P1 OR b = @P1 AND c = @P2"
    );
    assert_eq!(b.binds, ["id", "name"]);
    let b = bind_placeholders(sql, Driver::Clickhouse).unwrap();
    assert_eq!(
        b.sql,
        "SELECT * FROM t WHERE a = {p1:String} OR b = {p1:String} AND c = {p2:String}"
    );
    assert_eq!(b.binds, ["id", "name"]);
    let b = bind_placeholders(sql, Driver::Duckdb).unwrap();
    assert_eq!(b.sql, "SELECT * FROM t WHERE a = ? OR b = ? AND c = ?");
    assert_eq!(b.binds, ["id", "id", "name"]);
}

#[test]
fn reads_pass_writes_and_admin_dont() {
    for d in NEW {
        for sql in [
            "SELECT 1",
            "SELECT id, name FROM users WHERE id = 1",
            "WITH x AS (SELECT 1 AS a) SELECT a FROM x",
        ] {
            assert!(check_read_only(sql, d).is_ok(), "{d:?}: {sql}");
        }
        for sql in [
            "DELETE FROM users WHERE id = 1",
            "DROP TABLE users",
            "SELECT 1; SELECT 2",
        ] {
            assert!(check_read_only(sql, d).is_err(), "{d:?}: {sql}");
        }
    }
    assert!(check_read_only("SELECT TOP 5 [id] FROM [dbo].[users]", Driver::Mssql).is_ok());
    assert!(check_read_only(
        "SELECT * FROM users LIMIT 5 SETTINGS max_threads = 1",
        Driver::Clickhouse
    )
    .is_ok());
    assert!(check_read_only("SELECT * FROM users FORMAT CSV", Driver::Clickhouse).is_err());
    for sql in [
        "ATTACH 'x.db'",
        "INSTALL httpfs",
        "COPY users TO 'out.csv'",
        "SET threads = 1",
    ] {
        assert!(
            check_statement(sql, Driver::Duckdb).is_err_or_admin(),
            "{sql}"
        );
    }
}

trait AdminOrErr {
    fn is_err_or_admin(&self) -> bool;
}

impl AdminOrErr for Result<CheckedQuery, String> {
    fn is_err_or_admin(&self) -> bool {
        match self {
            Err(_) => true,
            Ok(q) => q.class == StatementClass::Admin,
        }
    }
}

#[test]
fn denied_functions_per_driver() {
    for (d, sql) in [
        (
            Driver::Clickhouse,
            "SELECT * FROM url('http://x/a.csv', CSV)",
        ),
        (
            Driver::Clickhouse,
            "SELECT * FROM file('/etc/passwd', 'LineAsString')",
        ),
        (
            Driver::Clickhouse,
            "SELECT * FROM remote('other:9000', db.t)",
        ),
        (Driver::Clickhouse, "SELECT dictGet('d', 'v', 1)"),
        (
            Driver::Mssql,
            "SELECT * FROM OPENROWSET('SQLNCLI', 'Server=x;', 'SELECT 1')",
        ),
        (Driver::Mssql, "SELECT * FROM OPENQUERY(other, 'SELECT 1')"),
        (Driver::Duckdb, "SELECT * FROM query('SELECT 1')"),
        (Driver::Duckdb, "SELECT getenv('HOME')"),
        (Driver::Duckdb, "SELECT * FROM sqlite_scan('x.db', 't')"),
    ] {
        let e = check_read_only(sql, d).unwrap_err();
        assert!(
            e.contains("not allowed") || e.contains("parse"),
            "{d:?}: {sql}: {e}"
        );
    }
}

#[test]
fn system_tables_per_driver() {
    assert!(is_system_table("system.users", Driver::Clickhouse));
    assert!(
        !is_system_table("system.users", Driver::Postgres),
        "only ClickHouse's"
    );
    assert!(is_system_table("sys.objects", Driver::Mssql));
    assert!(is_system_table("INFORMATION_SCHEMA.COLUMNS", Driver::Mssql));
    assert!(is_system_table("duckdb_settings", Driver::Duckdb));
    assert!(
        table_allowed("users", &["dbo.*".into()], Driver::Mssql),
        "dbo is the default schema"
    );
    assert!(table_allowed(
        "users",
        &["main.users".into()],
        Driver::Duckdb
    ));
}

#[test]
fn file_reads_are_collected() {
    let q = check_read_only(
        "SELECT * FROM read_csv('/data/a.csv', header = true) JOIN read_parquet(['/data/b.parquet', '/data/c/*.parquet']) USING (id)",
        Driver::Duckdb,
    )
    .unwrap();
    assert_eq!(
        check_file_reads(&q.statement).unwrap(),
        ["/data/a.csv", "/data/b.parquet", "/data/c/*.parquet"]
    );
    let q = check_read_only(
        "SELECT * FROM read_csv(concat('/data/', 'a.csv'))",
        Driver::Duckdb,
    )
    .unwrap();
    assert!(check_file_reads(&q.statement)
        .unwrap_err()
        .contains("quoted string"));
    assert!(file_table_name("data.csv"));
    assert!(file_table_name("/tmp/x"));
    assert!(!file_table_name("main.orders"));
}
