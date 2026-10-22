//! `:name` placeholder rewriting.

use forge_core::Driver;
use sqlparser::tokenizer::{Token, Tokenizer};

use super::dialect;

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
