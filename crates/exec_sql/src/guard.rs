//! Static checks on SQL before it reaches a database: placeholder binding and
//! the read-only AST guard (layers 3 and 4 of the read-only model).

use std::collections::{BTreeSet, HashSet};
use std::ops::ControlFlow;

use forge_core::{ColumnMask, Driver};
use sqlparser::ast::{
    Expr, FunctionArg, FunctionArgExpr, FunctionArguments, Ident, ObjectName, ObjectNamePart,
    Query, Select, SelectItem, SelectItemQualifiedWildcardKind, SetExpr, Statement, TableFactor,
    TableObject, Visit, Visitor,
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
    "cursor_to_xmlschema",
    // These read whole tables by name or run SQL from a string, which would
    // get around the table allowlist.
    "table_to_xml",
    "table_to_xmlschema",
    "table_to_xml_and_xmlschema",
    "schema_to_xml",
    "schema_to_xmlschema",
    "schema_to_xml_and_xmlschema",
    "database_to_xml",
    "database_to_xmlschema",
    "database_to_xml_and_xmlschema",
    "ts_stat",
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

/// What a statement does, from least to most privileged.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum StatementClass {
    #[default]
    Read,
    /// INSERT, UPDATE, DELETE, MERGE (and locking reads).
    Write,
    /// CREATE, ALTER, DROP, TRUNCATE, … (and SELECT … INTO).
    Ddl,
    /// Everything else: GRANT, SET, COPY, VACUUM, PRAGMA, CALL, … Never allowed.
    Admin,
}

impl StatementClass {
    pub fn as_str(self) -> &'static str {
        match self {
            StatementClass::Read => "read",
            StatementClass::Write => "write",
            StatementClass::Ddl => "ddl",
            StatementClass::Admin => "admin",
        }
    }
}

fn classify_one(s: &Statement) -> StatementClass {
    use StatementClass::*;
    match s {
        Statement::Query(_) => Read,
        Statement::Insert { .. }
        | Statement::Update { .. }
        | Statement::Delete { .. }
        | Statement::Merge { .. } => Write,
        Statement::CreateTable { .. }
        | Statement::CreateIndex { .. }
        | Statement::CreateView { .. }
        | Statement::CreateVirtualTable { .. }
        | Statement::CreateSchema { .. }
        | Statement::CreateSequence { .. }
        | Statement::CreateType { .. }
        | Statement::CreateDomain { .. }
        | Statement::CreateTrigger { .. }
        | Statement::CreateFunction { .. }
        | Statement::CreateProcedure { .. }
        | Statement::AlterTable { .. }
        | Statement::AlterIndex { .. }
        | Statement::AlterView { .. }
        | Statement::AlterType { .. }
        | Statement::AlterSchema { .. }
        | Statement::Drop { .. }
        | Statement::DropFunction { .. }
        | Statement::DropProcedure { .. }
        | Statement::DropTrigger { .. }
        | Statement::DropDomain { .. }
        | Statement::Truncate { .. }
        | Statement::RenameTable { .. }
        | Statement::Comment { .. } => Ddl,
        _ => Admin,
    }
}

/// Facts about a statement that passed the guard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedQuery {
    pub class: StatementClass,
    /// Why it isn't a plain read (for error messages), if it isn't.
    pub reason: Option<String>,
    /// A top-level write with `RETURNING`: its rows are the affected rows.
    pub returning: bool,
    /// UPDATE/DELETE without WHERE; refused by the gate once writes are allowed.
    pub missing_where: Option<String>,
    /// Tables and views the query reads (as written, possibly qualified).
    /// References to the query's own CTEs are not included.
    pub tables: Vec<String>,
    /// Table names and aliases usable as whole-row references.
    pub row_names: Vec<String>,
    pub statement: Statement,
}

/// CTE names visible inside one query, and those visible inside each of its
/// CTE bodies (a non-recursive CTE can't see itself or later siblings, so a
/// same-named reference there is the real table).
struct Scope {
    visible: HashSet<String>,
    ctes: Vec<(*const Query, HashSet<String>)>,
}

#[derive(Default)]
struct Guard {
    problem: Option<String>,
    class: StatementClass,
    reason: Option<String>,
    returning: bool,
    missing_where: Option<String>,
    depth: usize,
    tables: BTreeSet<String>,
    row_names: BTreeSet<String>,
    scopes: Vec<Scope>,
}

impl Guard {
    fn reject(&mut self, why: String) -> ControlFlow<()> {
        self.problem = Some(why);
        ControlFlow::Break(())
    }

    fn escalate(&mut self, class: StatementClass, reason: String) {
        if class > self.class {
            self.class = class;
        }
        if class != StatementClass::Read && self.reason.is_none() {
            self.reason = Some(reason);
        }
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
        let top = self.depth == 0;
        self.depth += 1;
        let class = classify_one(s);
        if class == StatementClass::Read {
            return ControlFlow::Continue(());
        }
        // Data-modifying CTEs (`WITH x AS (DELETE …) SELECT …`) would hide
        // their row count from the affected-rows cap.
        if !top {
            return self.reject(format!(
                "only SELECT queries are allowed; found {} inside the query. Run a write as its own statement",
                keyword(s)
            ));
        }
        match s {
            Statement::Update(u) if u.selection.is_none() => {
                self.missing_where = Some(
                    "UPDATE needs a WHERE clause; to change every row, say so with WHERE true"
                        .into(),
                );
            }
            Statement::Delete(d) if d.selection.is_none() => {
                self.missing_where = Some(
                    "DELETE needs a WHERE clause; to delete every row, say so with WHERE true"
                        .into(),
                );
            }
            Statement::Insert(i) => {
                if let TableObject::TableName(n) = &i.table {
                    self.tables.insert(full_name(n));
                }
                self.returning = i.returning.is_some();
            }
            Statement::Update(u) => self.returning = u.returning.is_some(),
            Statement::Delete(d) => self.returning = d.returning.is_some(),
            _ => {}
        }
        self.escalate(
            class,
            format!("only SELECT queries are allowed; found {}", keyword(s)),
        );
        ControlFlow::Continue(())
    }

    fn post_visit_statement(&mut self, _s: &Statement) -> ControlFlow<()> {
        self.depth -= 1;
        ControlFlow::Continue(())
    }

    fn pre_visit_query(&mut self, q: &Query) -> ControlFlow<()> {
        let base = match self.scopes.last() {
            Some(top) => top
                .ctes
                .iter()
                .find(|(p, _)| std::ptr::eq(*p, q))
                .map_or_else(|| top.visible.clone(), |(_, v)| v.clone()),
            None => HashSet::new(),
        };
        let mut scope = Scope {
            visible: base.clone(),
            ctes: Vec::new(),
        };
        if let Some(w) = &q.with {
            let aliases: Vec<String> = w
                .cte_tables
                .iter()
                .map(|c| c.alias.name.value.to_lowercase())
                .collect();
            for (i, c) in w.cte_tables.iter().enumerate() {
                let mut v = base.clone();
                v.extend(
                    if w.recursive {
                        &aliases[..]
                    } else {
                        &aliases[..i]
                    }
                    .iter()
                    .cloned(),
                );
                scope.ctes.push((&*c.query as *const Query, v));
            }
            scope.visible.extend(aliases);
        }
        self.scopes.push(scope);
        collect_table_exprs(&q.body, &mut self.tables);
        if !q.locks.is_empty() {
            self.escalate(
                StatementClass::Write,
                "locking clauses (FOR UPDATE/SHARE) are not allowed".into(),
            );
        }
        if select_into(&q.body) {
            self.escalate(StatementClass::Ddl, "SELECT … INTO is not allowed".into());
        }
        ControlFlow::Continue(())
    }

    fn post_visit_query(&mut self, _q: &Query) -> ControlFlow<()> {
        self.scopes.pop();
        ControlFlow::Continue(())
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
                name,
                alias,
                args: None,
                ..
            } => {
                self.row_names.insert(last_part(name).to_lowercase());
                if let Some(a) = alias {
                    self.row_names.insert(a.name.value.to_lowercase());
                }
                let is_cte = name.0.len() == 1
                    && self
                        .scopes
                        .last()
                        .is_some_and(|s| s.visible.contains(&last_part(name).to_lowercase()));
                if !is_cte {
                    self.tables.insert(full_name(name));
                }
                ControlFlow::Continue(())
            }
            TableFactor::Derived { alias: Some(a), .. } => {
                self.row_names.insert(a.name.value.to_lowercase());
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
    let q = check_statement(sql, driver)?;
    if q.class != StatementClass::Read {
        return Err(q
            .reason
            .unwrap_or_else(|| "only SELECT queries are allowed".into()));
    }
    Ok(q)
}

/// Parses and checks one statement of any class: single statement, no
/// denied function, no writes nested inside queries. Returns its class (and
/// a missing WHERE on UPDATE/DELETE) for the caller to gate.
pub fn check_statement(sql: &str, driver: Driver) -> Result<CheckedQuery, String> {
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
            class: guard.class,
            reason: guard.reason,
            returning: guard.returning,
            missing_where: guard.missing_where,
            tables: guard.tables.into_iter().collect(),
            row_names: guard.row_names.into_iter().collect(),
            statement: stmt.clone(),
        }),
    }
}

/// `TABLE name` reads a table without a FROM clause.
fn collect_table_exprs(body: &SetExpr, out: &mut BTreeSet<String>) {
    match body {
        SetExpr::Table(t) => {
            if let Some(name) = &t.table_name {
                out.insert(match &t.schema_name {
                    Some(s) => format!("{s}.{name}"),
                    None => name.clone(),
                });
            }
        }
        SetExpr::SetOperation { left, right, .. } => {
            collect_table_exprs(left, out);
            collect_table_exprs(right, out);
        }
        _ => {}
    }
}

/// `(schema, name)` of a table reference, lower-cased. `a.b.c` → `(a.b, c)`.
fn split_table(table: &str) -> (Option<String>, String) {
    let t = table.to_lowercase();
    match t.rsplit_once('.') {
        Some((s, n)) => (Some(s.to_owned()), n.to_owned()),
        None => (None, t),
    }
}

/// The schema an unqualified name resolves to, when it's fixed.
fn default_schema(driver: Driver) -> Option<&'static str> {
    match driver {
        Driver::Postgres => Some("public"),
        Driver::Sqlite => Some("main"),
        // The connection's database; not known without asking the server.
        Driver::Mysql => None,
    }
}

/// System catalogs: never readable through `run_sql`, whatever the allowlist.
pub fn is_system_table(table: &str, driver: Driver) -> bool {
    const SYSTEM_SCHEMAS: &[&str] = &[
        "pg_catalog",
        "information_schema",
        "pg_toast",
        "mysql",
        "performance_schema",
        "sys",
    ];
    let (schema, name) = split_table(table);
    match schema {
        Some(s) => {
            let last = s.rsplit('.').next().unwrap_or(&s);
            SYSTEM_SCHEMAS.contains(&last)
                || (driver == Driver::Sqlite && name.starts_with("sqlite_"))
        }
        // Unqualified pg_* names resolve to pg_catalog first.
        None => {
            (driver == Driver::Postgres && name.starts_with("pg_"))
                || (driver == Driver::Sqlite && name.starts_with("sqlite_"))
        }
    }
}

/// Whether `table` (as written in a query or passed to `describe_table`) is
/// covered by the allowlist. Entries: `*`, `schema.*`, `schema.table` or
/// `table` (any schema). An empty allowlist allows every non-system table.
/// Unqualified references match `schema.…` entries only for the driver's
/// fixed default schema (`public`, `main`).
pub fn table_allowed(table: &str, allowlist: &[String], driver: Driver) -> bool {
    if allowlist.is_empty() {
        return true;
    }
    let (schema, name) = split_table(table);
    let schema = schema.or_else(|| default_schema(driver).map(str::to_owned));
    allowlist.iter().any(|entry| {
        let entry = entry.to_lowercase();
        if entry == "*" {
            return true;
        }
        match entry.rsplit_once('.') {
            None => entry == name,
            Some((es, en)) => schema.as_deref() == Some(es) && (en == "*" || en == name),
        }
    })
}

/// Columns masked for a query that reads `tables`.
pub fn masked_columns(masks: &[ColumnMask], tables: &[String]) -> HashSet<String> {
    masks
        .iter()
        .filter(|m| m.applies(&m.column, tables))
        .map(|m| m.column.to_lowercase())
        .collect()
}

struct MaskGuard<'a> {
    columns: &'a HashSet<String>,
    row_names: HashSet<String>,
    top_select: Option<*const Select>,
    problem: Option<String>,
}

impl MaskGuard<'_> {
    fn reject(&mut self, why: String) -> ControlFlow<()> {
        self.problem = Some(why);
        ControlFlow::Break(())
    }
}

impl Visitor for MaskGuard<'_> {
    type Break = ();

    fn pre_visit_ident(&mut self, i: &Ident) -> ControlFlow<()> {
        if self.columns.contains(&i.value.to_lowercase()) {
            return self.reject(format!(
                "column `{}` is masked and can't be used in queries",
                i.value
            ));
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, e: &Expr) -> ControlFlow<()> {
        match e {
            // `to_json(u)` etc. would return the whole row, masked columns included.
            Expr::Identifier(i) if self.row_names.contains(&i.value.to_lowercase()) => self.reject(
                format!(
                    "whole-row references like `{}` aren't allowed while columns are masked; name the columns",
                    i.value
                ),
            ),
            Expr::QualifiedWildcard(..) => {
                self.reject("`t.*` inside expressions isn't allowed while columns are masked".into())
            }
            Expr::Function(f) => {
                let star = matches!(&f.args, FunctionArguments::List(l) if l.args.iter().any(|a| matches!(a,
                    FunctionArg::Unnamed(FunctionArgExpr::QualifiedWildcard(_))
                        | FunctionArg::Named { arg: FunctionArgExpr::QualifiedWildcard(_), .. }
                        | FunctionArg::ExprNamed { arg: FunctionArgExpr::QualifiedWildcard(_), .. })));
                if star {
                    self.reject(
                        "`t.*` inside function calls isn't allowed while columns are masked".into(),
                    )
                } else {
                    ControlFlow::Continue(())
                }
            }
            _ => ControlFlow::Continue(()),
        }
    }

    fn pre_visit_select(&mut self, s: &Select) -> ControlFlow<()> {
        let top = self.top_select.is_some_and(|t| std::ptr::eq(t, s));
        for item in &s.projection {
            match item {
                SelectItem::QualifiedWildcard(SelectItemQualifiedWildcardKind::Expr(_), _) => {
                    return self.reject("`(expr).*` isn't allowed while columns are masked".into());
                }
                // Masking works on result column names, which a nested or
                // set-operation `*` can rename; only the outermost SELECT may use it.
                SelectItem::Wildcard(_) | SelectItem::QualifiedWildcard(..) if !top => {
                    return self.reject(
                        "`*` is only allowed in the outermost SELECT while columns are masked; name the columns"
                            .into(),
                    );
                }
                _ => {}
            }
        }
        ControlFlow::Continue(())
    }
}

/// For agent-written SQL: refuses any way of reading a masked column other
/// than through a result column of the same name (which is then masked).
pub fn check_masks(q: &CheckedQuery, masks: &[ColumnMask]) -> Result<(), String> {
    let columns = masked_columns(masks, &q.tables);
    if columns.is_empty() {
        return Ok(());
    }
    let top_select = match &q.statement {
        Statement::Query(query) => match query.body.as_ref() {
            SetExpr::Select(s) => Some(&**s as *const Select),
            _ => None,
        },
        _ => None,
    };
    let mut g = MaskGuard {
        columns: &columns,
        row_names: q.row_names.iter().cloned().collect(),
        top_select,
        problem: None,
    };
    let _ = q.statement.visit(&mut g);
    g.problem.map_or(Ok(()), Err)
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
}
