//! Redis through the `redis` crate. Only the commands in the catalog run
//! (no EVAL, CONFIG, FLUSH*, KEYS, …). Commands that could return or walk a
//! huge value are bounded: HGETALL/SMEMBERS/HKEYS iterate with HSCAN/SSCAN
//! up to the row limit, and range commands are clamped or get COUNT/LIMIT.

use forge_exec_sql::{ConnectionReport, SqlError};
use redis::aio::MultiplexedConnection;
use redis::Value as Rv;
use serde_json::{json, Map, Value};

use crate::{Docs, Limits, Written};

#[derive(Clone)]
pub struct Conn {
    conn: MultiplexedConnection,
}

fn db_err(e: redis::RedisError) -> SqlError {
    if e.is_io_error() || e.is_connection_refusal() || e.is_connection_dropped() {
        SqlError::Connect(e.to_string())
    } else if e.is_timeout() {
        SqlError::Timeout(0)
    } else {
        SqlError::Database(e.to_string())
    }
}

/// Commands whose extra arguments are keys too.
const MULTI_KEY: &[&str] = &["MGET", "EXISTS", "DEL"];

/// Extra keys a command names besides its target.
pub(crate) fn extra_keys(op: &str, body: &Value) -> Vec<String> {
    if !MULTI_KEY.contains(&op.to_ascii_uppercase().as_str()) {
        return vec![];
    }
    args(body).unwrap_or_default()
}

/// The body's plain values as command arguments.
fn args(body: &Value) -> Result<Vec<String>, SqlError> {
    match body {
        Value::Null => Ok(vec![]),
        Value::Array(a) => a
            .iter()
            .map(|v| match v {
                Value::String(s) => Ok(s.clone()),
                Value::Number(n) => Ok(n.to_string()),
                Value::Bool(b) => Ok(if *b { "1" } else { "0" }.to_owned()),
                _ => Err(SqlError::Rejected(
                    "Redis arguments must be strings, numbers or booleans".into(),
                )),
            })
            .collect(),
        _ => Err(SqlError::Rejected(
            "the body must be an array of arguments".into(),
        )),
    }
}

/// A reply as JSON.
pub(crate) fn to_json(v: Rv) -> Value {
    match v {
        Rv::Nil => Value::Null,
        Rv::Int(i) => json!(i),
        Rv::Double(f) => {
            serde_json::Number::from_f64(f).map_or(Value::String(f.to_string()), Value::Number)
        }
        Rv::Boolean(b) => Value::Bool(b),
        Rv::Okay => json!("OK"),
        Rv::SimpleString(s) => Value::String(s),
        Rv::BulkString(b) => match String::from_utf8(b) {
            Ok(s) => Value::String(s),
            Err(e) => Value::String(format!("<binary, {} bytes>", e.as_bytes().len())),
        },
        Rv::VerbatimString { text, .. } => Value::String(text),
        Rv::Array(a) | Rv::Set(a) => Value::Array(a.into_iter().map(to_json).collect()),
        Rv::Map(m) => Value::Object(
            m.into_iter()
                .map(|(k, v)| {
                    let key = match to_json(k) {
                        Value::String(s) => s,
                        other => other.to_string(),
                    };
                    (key, to_json(v))
                })
                .collect(),
        ),
        other => Value::String(format!("{other:?}")),
    }
}

/// `start stop` clamped so at most `limit + 1` elements come back.
fn clamp_range(a: &mut [String], limit: u64) {
    if a.len() < 2 {
        return;
    }
    let (Ok(start), Ok(stop)) = (a[0].parse::<i64>(), a[1].parse::<i64>()) else {
        return;
    };
    if start < 0 {
        return;
    }
    let max_stop = start + limit as i64;
    if stop < 0 || stop > max_stop {
        a[1] = max_stop.to_string();
    }
}

impl Conn {
    /// `redis://[:password@]host:6379/0` or `rediss://` for TLS.
    pub(crate) async fn connect(dsn: &str, _timeout_ms: u32) -> Result<Self, SqlError> {
        let client = redis::Client::open(dsn.trim())
            .map_err(|e| SqlError::Config(format!("invalid connection string: {e}")))?;
        let conn = client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| SqlError::Connect(e.to_string()))?;
        Ok(Self { conn })
    }

    async fn cmd(&self, name: &str, args: &[String]) -> Result<Rv, SqlError> {
        let mut c = self.conn.clone();
        let mut cmd = redis::cmd(name);
        for a in args {
            cmd.arg(a);
        }
        cmd.query_async::<Rv>(&mut c).await.map_err(db_err)
    }

    /// HSCAN/SSCAN until `limit + 1` items or the end.
    async fn scan_all(
        &self,
        scan: &str,
        key: &str,
        limit: u64,
    ) -> Result<(Vec<Rv>, bool), SqlError> {
        let mut out = Vec::new();
        let mut cursor = "0".to_owned();
        let per = if scan == "HSCAN" { 2 } else { 1 };
        for _ in 0..1_000 {
            let reply = self
                .cmd(
                    scan,
                    &[key.into(), cursor.clone(), "COUNT".into(), "200".into()],
                )
                .await?;
            let Rv::Array(mut parts) = reply else { break };
            if parts.len() != 2 {
                break;
            }
            let items = parts.pop().unwrap_or(Rv::Nil);
            cursor = match parts.pop() {
                Some(Rv::BulkString(b)) => String::from_utf8_lossy(&b).into_owned(),
                _ => "0".into(),
            };
            if let Rv::Array(a) = items {
                out.extend(a);
            }
            if (out.len() / per) as u64 > limit {
                return Ok((out, true));
            }
            if cursor == "0" {
                break;
            }
        }
        Ok((out, false))
    }

    pub(crate) async fn read(
        &self,
        op: &str,
        key: &str,
        body: &Value,
        l: Limits,
    ) -> Result<Docs, SqlError> {
        let op = op.to_ascii_uppercase();
        let limit = u64::from(l.row_limit);
        let mut a = args(body)?;
        match op.as_str() {
            "HGETALL" | "HKEYS" => {
                let (items, truncated) = self.scan_all("HSCAN", key, limit).await?;
                let mut fields = Map::new();
                let mut keys = Vec::new();
                for pair in items.chunks(2).take(limit as usize) {
                    if let [k, v] = pair {
                        let k = match to_json(k.clone()) {
                            Value::String(s) => s,
                            other => other.to_string(),
                        };
                        keys.push(Value::String(k.clone()));
                        fields.insert(k, to_json(v.clone()));
                    }
                }
                let documents = if op == "HKEYS" {
                    keys
                } else if fields.is_empty() {
                    vec![]
                } else {
                    vec![Value::Object(fields)]
                };
                return Ok(Docs {
                    documents,
                    truncated,
                    summary: None,
                });
            }
            "SMEMBERS" => {
                let (items, truncated) = self.scan_all("SSCAN", key, limit).await?;
                let documents = items
                    .into_iter()
                    .take(limit as usize)
                    .map(to_json)
                    .collect();
                return Ok(Docs {
                    documents,
                    truncated,
                    summary: None,
                });
            }
            "LRANGE" | "ZRANGE" | "ZREVRANGE" => clamp_range(&mut a, limit),
            "XRANGE" | "XREVRANGE" if !a.iter().any(|x| x.eq_ignore_ascii_case("COUNT")) => {
                a.extend(["COUNT".into(), (limit + 1).to_string()]);
            }
            "ZRANGEBYSCORE" if !a.iter().any(|x| x.eq_ignore_ascii_case("LIMIT")) => {
                a.extend(["LIMIT".into(), "0".into(), (limit + 1).to_string()]);
            }
            _ => {}
        }
        let mut full = vec![key.to_owned()];
        full.extend(a);
        let reply = to_json(self.cmd(&op, &full).await?);
        let (documents, truncated) = match reply {
            Value::Array(items) => {
                let t = items.len() as u64 > limit;
                (items.into_iter().take(limit as usize).collect(), t)
            }
            Value::Null => (vec![], false),
            other => (vec![other], false),
        };
        Ok(Docs {
            documents,
            truncated,
            summary: None,
        })
    }

    pub(crate) async fn write(
        &self,
        op: &str,
        key: &str,
        body: &Value,
        l: Limits,
        dry_run: bool,
    ) -> Result<Written, SqlError> {
        let op = op.to_ascii_uppercase();
        let a = args(body)?;
        let keys = 1 + extra_keys(&op, body).len() as u64;
        if keys > u64::from(l.max_affected) {
            return Err(SqlError::TooManyRows {
                affected: keys,
                max: l.max_affected,
                at_least: false,
            });
        }
        if dry_run {
            return Ok(Written {
                affected: keys,
                committed: false,
                documents: vec![],
            });
        }
        let mut full = vec![key.to_owned()];
        full.extend(a);
        let reply = to_json(self.cmd(&op, &full).await?);
        let affected = match (&*op, &reply) {
            ("DEL", Value::Number(n)) => n.as_u64().unwrap_or(0),
            _ => 1,
        };
        Ok(Written {
            affected,
            committed: true,
            documents: vec![json!({ "reply": reply })],
        })
    }

    /// Keys matching `pattern` (a Redis glob), via SCAN.
    pub(crate) async fn list_targets(
        &self,
        pattern: Option<&str>,
        max: usize,
    ) -> Result<Vec<String>, SqlError> {
        let pattern = pattern.unwrap_or("*");
        let mut out = Vec::new();
        let mut cursor = "0".to_owned();
        for _ in 0..200 {
            let reply = self
                .cmd(
                    "SCAN",
                    &[
                        cursor.clone(),
                        "MATCH".into(),
                        pattern.into(),
                        "COUNT".into(),
                        "500".into(),
                    ],
                )
                .await?;
            let Rv::Array(mut parts) = reply else { break };
            if parts.len() != 2 {
                break;
            }
            let keys = parts.pop().unwrap_or(Rv::Nil);
            cursor = match parts.pop() {
                Some(Rv::BulkString(b)) => String::from_utf8_lossy(&b).into_owned(),
                _ => "0".into(),
            };
            if let Value::Array(ks) = to_json(keys) {
                out.extend(ks.into_iter().filter_map(|k| k.as_str().map(str::to_owned)));
            }
            if out.len() > max || cursor == "0" {
                break;
            }
        }
        Ok(out)
    }

    /// Type, TTL and size of one key.
    pub(crate) async fn describe(&self, key: &str) -> Result<Value, SqlError> {
        let ty = match to_json(self.cmd("TYPE", &[key.into()]).await?) {
            Value::String(s) => s,
            other => other.to_string(),
        };
        let ttl = to_json(self.cmd("TTL", &[key.into()]).await?);
        let size_cmd = match ty.as_str() {
            "string" => Some("STRLEN"),
            "hash" => Some("HLEN"),
            "list" => Some("LLEN"),
            "set" => Some("SCARD"),
            "zset" => Some("ZCARD"),
            "stream" => Some("XLEN"),
            _ => None,
        };
        let size = match size_cmd {
            Some(c) => to_json(self.cmd(c, &[key.into()]).await?),
            None => Value::Null,
        };
        Ok(json!({ "key": key, "type": ty, "ttl_seconds": ttl, "size": size }))
    }

    pub(crate) async fn probe(&self) -> Result<ConnectionReport, SqlError> {
        let info = match to_json(self.cmd("INFO", &["server".into()]).await?) {
            Value::String(s) => s,
            _ => String::new(),
        };
        let version = info
            .lines()
            .find_map(|l| l.strip_prefix("redis_version:"))
            .map(|v| format!("Redis {}", v.trim()));
        let mut r = ConnectionReport {
            server_version: version,
            ..Default::default()
        };
        let user = match self.cmd("ACL", &["WHOAMI".into()]).await.map(to_json) {
            Ok(Value::String(u)) => Some(u),
            _ => None,
        };
        r.user = user.clone();
        match user {
            Some(u) => match self
                .cmd(
                    "ACL",
                    &[
                        "DRYRUN".into(),
                        u,
                        "SET".into(),
                        "mcp-forge:probe".into(),
                        "x".into(),
                    ],
                )
                .await
                .map(to_json)
            {
                Ok(Value::String(s)) if s == "OK" => {
                    r.can_write = Some(true);
                    r.write_privileges
                        .push("this user may run SET (and likely other writes)".into());
                }
                Ok(_) => r.can_write = Some(false),
                Err(_) => r
                    .warnings
                    .push("Couldn't check the user's ACL (Redis 7+ only).".into()),
            },
            None => r.warnings.push("Couldn't read the ACL user.".into()),
        }
        Ok(r)
    }
}
