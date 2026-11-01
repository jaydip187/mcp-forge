//! ClickHouse over its HTTP interface. Every query is sent with
//! `readonly=2` (reads only; limits may still be set per query), a
//! server-side `max_execution_time` and a result-row cap. ClickHouse has no
//! transactions, so connections can't be given write access.

use std::time::Duration;

use reqwest::Url;
use serde::Deserialize;
use serde_json::Value;

use crate::{Bind, Limits, RawResult, SqlError};

/// Connection details parsed from `http(s)://user:password@host:8123/database`.
#[derive(Clone)]
pub struct ClickhousePool {
    client: reqwest::Client,
    /// Without credentials or path.
    base: Url,
    user: Option<String>,
    password: Option<String>,
    database: Option<String>,
}

impl std::fmt::Debug for ClickhousePool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClickhousePool")
            .field("base", &self.base.as_str())
            .field("database", &self.database)
            .finish_non_exhaustive()
    }
}

impl ClickhousePool {
    pub(crate) fn new(dsn: &str, timeout_ms: u32) -> Result<Self, SqlError> {
        let dsn = dsn.trim();
        let rewritten = if let Some(rest) = dsn.strip_prefix("clickhouse://") {
            format!("http://{rest}")
        } else {
            dsn.to_owned()
        };
        let url = Url::parse(&rewritten)
            .map_err(|e| SqlError::Config(format!("invalid connection string: {e}")))?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return Err(SqlError::Config(
                "the connection string must look like https://user:password@host:8443/database"
                    .into(),
            ));
        }
        let decode = |s: &str| crate::error::urldecode(s).unwrap_or_else(|_| s.to_owned());
        let user = (!url.username().is_empty()).then(|| decode(url.username()));
        let password = url.password().map(decode);
        let database = url
            .path_segments()
            .and_then(|mut p| p.next().map(str::to_owned))
            .filter(|d| !d.is_empty());
        let mut base = url.clone();
        base.set_path("/");
        base.set_query(None);
        base.set_fragment(None);
        let _ = base.set_username("");
        let _ = base.set_password(None);
        let client = reqwest::Client::builder()
            .connect_timeout(
                Duration::from_millis(u64::from(timeout_ms)).max(Duration::from_secs(5)),
            )
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| SqlError::Config(format!("HTTP client: {e}")))?;
        Ok(Self {
            client,
            base,
            user,
            password,
            database,
        })
    }

    /// Sends one statement; returns the response text.
    async fn send(&self, sql: &str, params: &[(String, String)]) -> Result<String, SqlError> {
        let mut url = self.base.clone();
        url.query_pairs_mut().extend_pairs(params);
        let mut req = self.client.post(url).body(sql.to_owned());
        if let Some(u) = &self.user {
            req = req.header("X-ClickHouse-User", u);
        }
        if let Some(p) = &self.password {
            req = req.header("X-ClickHouse-Key", p);
        }
        if let Some(d) = &self.database {
            req = req.header("X-ClickHouse-Database", d);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| SqlError::Connect(e.without_url().to_string()))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| SqlError::Connect(e.without_url().to_string()))?;
        if status.is_success() {
            return Ok(text);
        }
        // `Code: 159. DB::Exception: Timeout exceeded: …` (TIMEOUT_EXCEEDED).
        if text.contains("Code: 159.") {
            return Err(SqlError::Timeout(0));
        }
        let first = text.lines().next().unwrap_or("").trim();
        let msg = first
            .split_once("DB::Exception: ")
            .map_or(first, |(_, m)| m)
            .trim_end_matches(|c: char| c == '.' || c.is_whitespace());
        if matches!(status.as_u16(), 401 | 403) || text.contains("Code: 516.") {
            return Err(SqlError::Connect(msg.to_owned()));
        }
        Err(SqlError::Database(msg.to_owned()))
    }
}

/// The ClickHouse type a bind is sent as.
fn type_of(b: &Bind) -> &'static str {
    match b {
        Bind::Null("integer") => "Nullable(Int64)",
        Bind::Null("number") => "Nullable(Float64)",
        Bind::Null("boolean") => "Nullable(Bool)",
        Bind::Null(_) => "Nullable(String)",
        Bind::Bool(_) => "Bool",
        Bind::Int(_) => "Int64",
        Bind::Float(_) => "Float64",
        Bind::Text(_) => "String",
    }
}

/// HTTP parameter value in ClickHouse's escaped text format.
fn param_value(b: &Bind) -> String {
    match b {
        Bind::Null(_) => "\\N".into(),
        Bind::Bool(v) => v.to_string(),
        Bind::Int(v) => v.to_string(),
        Bind::Float(v) => v.to_string(),
        Bind::Text(s) => s
            .replace('\\', "\\\\")
            .replace('\t', "\\t")
            .replace('\n', "\\n"),
    }
}

/// The binder writes `{pN:String}` for every placeholder; each gets its
/// value's type here, and the values go as `param_pN`.
pub(crate) fn typed(sql: &str, binds: &[Bind]) -> (String, Vec<(String, String)>) {
    let mut out = sql.to_owned();
    let mut params = Vec::with_capacity(binds.len());
    for (i, b) in binds.iter().enumerate() {
        let n = i + 1;
        out = out.replace(
            &format!("{{p{n}:String}}"),
            &format!("{{p{n}:{}}}", type_of(b)),
        );
        params.push((format!("param_p{n}"), param_value(b)));
    }
    (out, params)
}

#[derive(Deserialize)]
struct Meta {
    name: String,
}

#[derive(Deserialize)]
struct Compact {
    #[serde(default)]
    meta: Vec<Meta>,
    #[serde(default)]
    data: Vec<Vec<Value>>,
}

pub(crate) async fn run_read_only(
    pool: &ClickhousePool,
    sql: &str,
    binds: &[Bind],
    limits: Limits,
) -> Result<RawResult, SqlError> {
    let (sql, mut params) = typed(sql, binds);
    let secs = f64::from(limits.timeout_ms) / 1000.0;
    params.extend(
        [
            ("readonly", "2".to_owned()),
            ("max_execution_time", secs.to_string()),
            (
                "max_result_rows",
                (u64::from(limits.row_limit) + 1).to_string(),
            ),
            ("result_overflow_mode", "break".to_owned()),
            ("default_format", "JSONCompact".to_owned()),
            ("output_format_json_quote_64bit_integers", "0".to_owned()),
            ("output_format_json_named_tuples_as_objects", "1".to_owned()),
        ]
        .map(|(k, v)| (k.to_owned(), v)),
    );
    let text = pool.send(&sql, &params).await?;
    let parsed: Compact = serde_json::from_str(&text).map_err(|_| {
        SqlError::Database("ClickHouse didn't return JSON; remove any FORMAT clause".into())
    })?;
    let limit = limits.row_limit as usize;
    let truncated = parsed.data.len() > limit;
    let rows = parsed
        .data
        .into_iter()
        .take(limit)
        .map(|r| r.into_iter().map(crate::value::cap_cell).collect())
        .collect();
    Ok(RawResult {
        columns: parsed.meta.into_iter().map(|m| m.name).collect(),
        rows,
        truncated,
    })
}

/// Server version, user and whether the user may write.
pub(crate) async fn probe(
    pool: &ClickhousePool,
) -> Result<crate::pool::ConnectionReport, SqlError> {
    let limits = Limits {
        row_limit: 10,
        timeout_ms: 10_000,
    };
    let raw = run_read_only(
        pool,
        "SELECT version(), currentUser(), getSetting('readonly')",
        &[],
        limits,
    )
    .await?;
    let row = raw.rows.first().cloned().unwrap_or_default();
    let text = |i: usize| row.get(i).and_then(Value::as_str).map(str::to_owned);
    let mut r = crate::pool::ConnectionReport {
        server_version: text(0).map(|v| format!("ClickHouse {v}")),
        user: text(1),
        ..Default::default()
    };
    // Grants with write power, from the user's own view of system.grants.
    let grants = run_read_only(
        pool,
        "SELECT DISTINCT access_type FROM system.grants \
         WHERE user_name = currentUser() OR role_name IN (SELECT role_name FROM system.current_roles)",
        &[],
        Limits {
            row_limit: 200,
            timeout_ms: 10_000,
        },
    )
    .await;
    match grants {
        Ok(g) => {
            const WRITE: &[&str] = &[
                "ALL", "INSERT", "ALTER", "CREATE", "DROP", "TRUNCATE", "OPTIMIZE",
            ];
            r.write_privileges = g
                .rows
                .iter()
                .filter_map(|row| row.first().and_then(Value::as_str))
                .filter(|a| WRITE.iter().any(|w| a.starts_with(w)))
                .map(str::to_owned)
                .collect();
            r.can_write = Some(!r.write_privileges.is_empty());
        }
        Err(_) => r.warnings.push(
            "Couldn't read the user's grants; mcp-forge still sends every query read-only.".into(),
        ),
    }
    Ok(r)
}

#[cfg(test)]
#[path = "../tests/unit/clickhouse.rs"]
mod tests;
