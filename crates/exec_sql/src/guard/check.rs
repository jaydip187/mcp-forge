//! The AST walk behind the read-only and statement checks.

use std::collections::{BTreeSet, HashSet};
use std::ops::ControlFlow;

use forge_core::Driver;
use sqlparser::ast::{Expr, Query, SetExpr, Statement, TableFactor, TableObject, Visit, Visitor};
use sqlparser::parser::Parser;

use super::{
    classify::classify_one, classify::StatementClass, deny::denied, deny::full_name,
    deny::last_part, dialect,
};

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
