//! Static checks on SQL before it reaches a database: placeholder binding and
//! the read-only AST guard (layers 3 and 4 of the read-only model).

use std::collections::BTreeSet;
use std::ops::ControlFlow;

use forge_core::Driver;
use sqlparser::ast::{
    Expr, ObjectName, ObjectNamePart, Query, SetExpr, Statement, TableFactor, Visit, Visitor,
};
use sqlparser::dialect::{Dialect, MySqlDialect, PostgreSqlDialect, SQLiteDialect};
use sqlparser::parser::Parser;
use sqlparser::tokenizer::{Token, Tokenizer};

fn dialect(driver: Driver) -> Box<dyn Dialect> {
    match driver {
        Driver::Sqlite => Box::new(SQLiteDialect {}),
        Driver::Postgres => Box::new(PostgreSqlDialect {}),
        Driver::Mysql => Box::new(MySqlDialect {}),
    }
}

/// SQL with `:name` placeholders rewritten to the driver's positional form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundSql {
    pub sql: String,
    /// Param name for each positional bind, in bind order.
    pub binds: Vec<String>,
    /// Distinct placeholder names, in order of first use.
    pub names: Vec<String>,
}

/// Rewrites `:name` placeholders (outside strings, comments and `::` casts)
/// to `$n` (Postgres; a repeated name reuses its number) or `?` (SQLite,
/// MySQL; one bind per occurrence). The rest of the text is kept verbatim.
pub fn bind_placeholders(sql: &str, driver: Driver) -> Result<BoundSql, String> {
    let d = dialect(driver);
    let tokens = Tokenizer::new(d.as_ref(), sql)
        .tokenize_with_location()
        .map_err(|e| format!("could not read the SQL: {e}"))?;
    if tokens
        .iter()
        .any(|t| matches!(t.token, Token::Placeholder(_)))
    {
        return Err("use `:name` placeholders, not `?` or `$1`".into());
    }

    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(sql.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    let offset = |line: u64, col: u64| -> usize {
        let start = line_starts[(line as usize).saturating_sub(1)];
        sql[start..]
            .char_indices()
            .nth((col as usize).saturating_sub(1))
            .map_or(sql.len(), |(i, _)| start + i)
    };

    let mut out = String::with_capacity(sql.len());
    let mut copied = 0;
    let mut names: Vec<String> = Vec::new();
    let mut binds = Vec::new();
    for pair in tokens.windows(2) {
        let (Token::Colon, Token::Word(w)) = (&pair[0].token, &pair[1].token) else {
            continue;
        };
        let (c, n) = (pair[0].span.start, pair[1].span.start);
        if w.quote_style.is_some() || n.line != c.line || n.column != c.column + 1 {
            continue;
        }
        let start = offset(c.line, c.column);
        let end = start + 1 + w.value.len();
        if sql.get(start + 1..end) != Some(w.value.as_str()) {
            continue;
        }
        out.push_str(&sql[copied..start]);
        let idx = match names.iter().position(|x| x == &w.value) {
            Some(i) => i,
            None => {
                names.push(w.value.clone());
                names.len() - 1
            }
        };
        match driver {
            Driver::Postgres => out.push_str(&format!("${}", idx + 1)),
            Driver::Sqlite | Driver::Mysql => {
                out.push('?');
                binds.push(w.value.clone());
            }
        }
        copied = end;
    }
    out.push_str(&sql[copied..]);
    if driver == Driver::Postgres {
        binds = names.clone();
    }
    Ok(BoundSql {
        sql: out,
        binds,
        names,
    })
}

/// Functions that can write, reach outside the database, read server files,
/// take locks, or stall the server. Matched on the unqualified name.
const DENIED_FUNCTIONS: &[&str] = &[
    // Postgres
    "pg_terminate_backend",
    "pg_cancel_backend",
    "pg_reload_conf",
    "pg_rotate_logfile",
    "pg_read_file",
    "pg_read_binary_file",
    "pg_ls_dir",
    "pg_stat_file",
    "pg_sleep",
    "pg_sleep_for",
    "pg_sleep_until",
    "pg_switch_wal",
    "pg_create_restore_point",
    "pg_logical_emit_message",
    "pg_notify",
    "set_config",
    "nextval",
    "setval",
    "query_to_xml",
    "query_to_xmlschema",
    "query_to_xml_and_xmlschema",
    "cursor_to_xml",
    // MySQL
    "sleep",
    "benchmark",
    "load_file",
    "get_lock",
    "release_lock",
    "release_all_locks",
    "master_pos_wait",
    "source_pos_wait",
    // SQLite
    "load_extension",
    "readfile",
    "writefile",
    "edit",
];

/// Name prefixes denied as families (`dblink_exec`, `lo_import`, …).
const DENIED_PREFIXES: &[&str] = &["dblink", "lo_", "pg_advisory", "pg_try_advisory"];

fn denied(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    DENIED_FUNCTIONS.contains(&n.as_str()) || DENIED_PREFIXES.iter().any(|p| n.starts_with(p))
}

fn last_part(name: &ObjectName) -> String {
    match name.0.last() {
        Some(ObjectNamePart::Identifier(i)) => i.value.clone(),
        Some(ObjectNamePart::Function(f)) => f.name.value.clone(),
        None => String::new(),
    }
}

fn full_name(name: &ObjectName) -> String {
    name.0
        .iter()
        .map(|p| match p {
            ObjectNamePart::Identifier(i) => i.value.clone(),
            ObjectNamePart::Function(f) => f.name.value.clone(),
        })
        .collect::<Vec<_>>()
        .join(".")
}

/// Facts about a statement that passed the guard.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckedQuery {
    /// Tables and views the query reads (as written, possibly qualified).
    pub tables: Vec<String>,
}

#[derive(Default)]
struct Guard {
    problem: Option<String>,
    tables: BTreeSet<String>,
}

impl Guard {
    fn reject(&mut self, why: String) -> ControlFlow<()> {
        self.problem = Some(why);
        ControlFlow::Break(())
    }
}

/// First word of a statement, e.g. `DELETE`.
fn keyword(s: &Statement) -> String {
    s.to_string()
        .split_whitespace()
        .next()
        .unwrap_or("statement")
        .to_uppercase()
}

fn select_into(body: &SetExpr) -> bool {
    match body {
        SetExpr::Select(s) => s.into.is_some(),
        SetExpr::SetOperation { left, right, .. } => select_into(left) || select_into(right),
        _ => false,
    }
}

impl Visitor for Guard {
    type Break = ();

    fn pre_visit_statement(&mut self, s: &Statement) -> ControlFlow<()> {
        match s {
            Statement::Query(_) => ControlFlow::Continue(()),
            // Also catches data-modifying CTEs: `WITH x AS (DELETE …) SELECT …`.
            other => self.reject(format!(
                "only SELECT queries are allowed; found {}",
                keyword(other)
            )),
        }
    }

    fn pre_visit_query(&mut self, q: &Query) -> ControlFlow<()> {
        if !q.locks.is_empty() {
            return self.reject("locking clauses (FOR UPDATE/SHARE) are not allowed".into());
        }
        if select_into(&q.body) {
            return self.reject("SELECT … INTO is not allowed".into());
        }
        match q.body.as_ref() {
            SetExpr::Insert(s) | SetExpr::Update(s) | SetExpr::Delete(s) | SetExpr::Merge(s) => {
                self.reject(format!(
                    "only SELECT queries are allowed; found {}",
                    keyword(s)
                ))
            }
            _ => ControlFlow::Continue(()),
        }
    }

    fn pre_visit_table_factor(&mut self, t: &TableFactor) -> ControlFlow<()> {
        match t {
            TableFactor::Table {
                name,
                args: Some(_),
                ..
            }
            | TableFactor::Function { name, .. }
                if denied(&last_part(name)) =>
            {
                self.reject(format!("function `{}` is not allowed", full_name(name)))
            }
            TableFactor::Table {
                name, args: None, ..
            } => {
                self.tables.insert(full_name(name));
                ControlFlow::Continue(())
            }
            _ => ControlFlow::Continue(()),
        }
    }

    fn pre_visit_expr(&mut self, e: &Expr) -> ControlFlow<()> {
        if let Expr::Function(f) = e {
            if denied(&last_part(&f.name)) {
                return self.reject(format!("function `{}` is not allowed", full_name(&f.name)));
            }
        }
        ControlFlow::Continue(())
    }
}

/// Accepts exactly one read-only query: a `SELECT`/`WITH`/`VALUES`/`TABLE`
/// statement with no data-modifying part anywhere in its tree, no locking
/// or `INTO` clause, and no denied function.
pub fn check_read_only(sql: &str, driver: Driver) -> Result<CheckedQuery, String> {
    let d = dialect(driver);
    let statements =
        Parser::parse_sql(d.as_ref(), sql).map_err(|e| format!("could not parse the SQL: {e}"))?;
    let stmt = match statements.as_slice() {
        [] => return Err("the SQL is empty".into()),
        [one] => one,
        _ => return Err("only a single statement is allowed".into()),
    };
    let mut guard = Guard::default();
    let _ = stmt.visit(&mut guard);
    match guard.problem {
        Some(p) => Err(p),
        None => Ok(CheckedQuery {
            tables: guard.tables.into_iter().collect(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            let b = bind_placeholders("SELECT :a, :b, :a, 'é' || :b", d).unwrap();
            assert_eq!(b.sql, "SELECT ?, ?, ?, 'é' || ?");
            assert_eq!(b.binds, ["a", "b", "a", "b"]);
            assert_eq!(b.names, ["a", "b"]);
        }
        // Multi-line with non-ASCII before the placeholder.
        let b = bind_placeholders("SELECT 'ü'\n  , x FROM t WHERE y = :y", Driver::Sqlite).unwrap();
        assert_eq!(b.sql, "SELECT 'ü'\n  , x FROM t WHERE y = ?");
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
}
