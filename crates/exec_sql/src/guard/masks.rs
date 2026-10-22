//! Column masks for agent-written SQL.

use std::collections::HashSet;
use std::ops::ControlFlow;

use forge_core::ColumnMask;
use sqlparser::ast::{
    Expr, FunctionArg, FunctionArgExpr, FunctionArguments, Ident, Select, SelectItem,
    SelectItemQualifiedWildcardKind, SetExpr, Statement, Visit, Visitor,
};

use super::check::CheckedQuery;

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
