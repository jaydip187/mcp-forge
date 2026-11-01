//! DuckDB file reads: `read_csv('…')` and friends. The guard only collects
//! their paths; the connection's allowed folders decide (see `policy`), and
//! DuckDB itself is opened with `allowed_directories` as well.

use std::ops::ControlFlow;

use sqlparser::ast::{
    Expr, FunctionArg, FunctionArgExpr, FunctionArguments, Statement, TableFactor, Value, Visit,
    Visitor,
};

use super::deny::last_part;

/// Table functions whose first argument is a file path, a glob or a list of
/// them.
pub const FILE_FUNCTIONS: &[&str] = &[
    "read_csv",
    "read_csv_auto",
    "sniff_csv",
    "read_parquet",
    "parquet_scan",
    "parquet_metadata",
    "parquet_schema",
    "parquet_file_metadata",
    "parquet_kv_metadata",
    "read_json",
    "read_json_auto",
    "read_json_objects",
    "read_json_objects_auto",
    "read_ndjson",
    "read_ndjson_auto",
    "read_ndjson_objects",
    "read_text",
    "read_blob",
    "read_xlsx",
    "glob",
    "delta_scan",
    "iceberg_scan",
];

fn is_file_function(name: &str) -> bool {
    FILE_FUNCTIONS.contains(&name.to_ascii_lowercase().as_str())
}

/// File extensions DuckDB reads when a table name is a path (`FROM 'x.csv'`).
const FILE_EXTENSIONS: &[&str] = &[
    ".csv", ".tsv", ".txt", ".parquet", ".json", ".jsonl", ".ndjson", ".xlsx", ".gz", ".zst",
];

/// Whether a table reference is really a file path (DuckDB's replacement
/// scans). These must go through a `read_*` function instead.
pub fn file_table_name(table: &str) -> bool {
    let t = table.to_ascii_lowercase();
    t.contains('/') || t.contains('\\') || FILE_EXTENSIONS.iter().any(|e| t.ends_with(e))
}

#[derive(Default)]
struct Files {
    paths: Vec<String>,
    problem: Option<String>,
}

impl Files {
    fn read(&mut self, name: &str, args: &[FunctionArg]) -> ControlFlow<()> {
        let first = args.iter().find_map(|a| match a {
            FunctionArg::Unnamed(FunctionArgExpr::Expr(e)) => Some(e),
            _ => None,
        });
        let literal = |e: &Expr| match e {
            Expr::Value(v) => match &v.value {
                Value::SingleQuotedString(s) | Value::DoubleQuotedString(s) => Some(s.clone()),
                _ => None,
            },
            _ => None,
        };
        let found: Option<Vec<String>> = match first {
            Some(Expr::Array(a)) => a.elem.iter().map(literal).collect(),
            Some(e) => literal(e).map(|p| vec![p]),
            None => None,
        };
        match found {
            Some(ps) if !ps.is_empty() => {
                self.paths.extend(ps);
                ControlFlow::Continue(())
            }
            _ => {
                self.problem = Some(format!(
                    "`{name}` needs the file path as a quoted string (or a list of them)"
                ));
                ControlFlow::Break(())
            }
        }
    }
}

impl Visitor for Files {
    type Break = ();

    fn pre_visit_table_factor(&mut self, t: &TableFactor) -> ControlFlow<()> {
        match t {
            TableFactor::Table {
                name,
                args: Some(a),
                ..
            } if is_file_function(&last_part(name)) => self.read(&last_part(name), &a.args),
            TableFactor::Function { name, args, .. } if is_file_function(&last_part(name)) => {
                self.read(&last_part(name), args)
            }
            _ => ControlFlow::Continue(()),
        }
    }

    fn pre_visit_expr(&mut self, e: &Expr) -> ControlFlow<()> {
        match e {
            Expr::Function(f) if is_file_function(&last_part(&f.name)) => match &f.args {
                FunctionArguments::List(l) => self.read(&last_part(&f.name), &l.args),
                _ => self.read(&last_part(&f.name), &[]),
            },
            _ => ControlFlow::Continue(()),
        }
    }
}

/// Paths read by file functions in `stmt`, in order. Fails when a path isn't
/// a literal, since it couldn't be checked before running.
pub fn check_file_reads(stmt: &Statement) -> Result<Vec<String>, String> {
    let mut f = Files::default();
    let _ = stmt.visit(&mut f);
    match f.problem {
        Some(p) => Err(p),
        None => Ok(f.paths),
    }
}
