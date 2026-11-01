//! Elasticsearch (and OpenSearch) over HTTP. Searches get a server-side
//! `timeout` and a `size` capped at the row limit; index names and ids are
//! sent as encoded path segments; script keys are refused by the policy.

use std::time::Duration;

use forge_exec_sql::{ConnectionReport, SqlError};
use reqwest::{Method, Url};
use serde_json::{json, Map, Value};

use crate::{Docs, Limits, Written};

/// `https://user:password@host:9200` or `https://host:9200?api_key=KEY`.
#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base: Url,
    auth: Auth,
}

#[derive(Clone)]
enum Auth {
    None,
    Basic(String, String),
    ApiKey(String),
}

impl Client {
    pub(crate) fn new(dsn: &str, timeout_ms: u32) -> Result<Self, SqlError> {
        let dsn = dsn.trim();
        let rewritten = dsn
            .strip_prefix("elasticsearch://")
            .map(|r| format!("https://{r}"))
            .unwrap_or_else(|| dsn.to_owned());
        let url = Url::parse(&rewritten)
            .map_err(|e| SqlError::Config(format!("invalid connection string: {e}")))?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return Err(SqlError::Config(
                "the connection string must look like https://user:password@host:9200".into(),
            ));
        }
        let decode = |s: &str| forge_exec_sql_urldecode(s);
        let api_key = url
            .query_pairs()
            .find(|(k, _)| k == "api_key")
            .map(|(_, v)| v.into_owned());
        let auth = match (api_key, url.username()) {
            (Some(k), _) => Auth::ApiKey(k),
            (None, "") => Auth::None,
            (None, u) => Auth::Basic(decode(u), url.password().map(decode).unwrap_or_default()),
        };
        let mut base = url.clone();
        base.set_query(None);
        base.set_fragment(None);
        let _ = base.set_username("");
        let _ = base.set_password(None);
        let http = reqwest::Client::builder()
            .connect_timeout(
                Duration::from_millis(u64::from(timeout_ms)).max(Duration::from_secs(5)),
            )
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| SqlError::Config(format!("HTTP client: {e}")))?;
        Ok(Self { http, base, auth })
    }

    /// `base` + encoded path segments + query.
    fn url(&self, segments: &[&str], query: &[(&str, String)]) -> Url {
        let mut u = self.base.clone();
        {
            let mut p = u.path_segments_mut().expect("http URL");
            p.pop_if_empty();
            for s in segments {
                p.push(s);
            }
        }
        if !query.is_empty() {
            u.query_pairs_mut()
                .extend_pairs(query.iter().map(|(k, v)| (*k, v.as_str())));
        }
        u
    }

    /// Sends a request; returns the status and JSON body.
    async fn send(
        &self,
        method: Method,
        url: Url,
        body: Option<&Value>,
    ) -> Result<(u16, Value), SqlError> {
        let mut req = self.http.request(method, url);
        req = match &self.auth {
            Auth::None => req,
            Auth::Basic(u, p) => req.basic_auth(u, Some(p)),
            Auth::ApiKey(k) => req.header("Authorization", format!("ApiKey {k}")),
        };
        if let Some(b) = body {
            req = req.json(b);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| SqlError::Connect(e.without_url().to_string()))?;
        let status = resp.status().as_u16();
        let text = resp
            .text()
            .await
            .map_err(|e| SqlError::Connect(e.without_url().to_string()))?;
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::String(text));
        Ok((status, v))
    }

    /// Fails on error statuses with Elasticsearch's own reason.
    async fn call(
        &self,
        method: Method,
        url: Url,
        body: Option<&Value>,
    ) -> Result<Value, SqlError> {
        let (status, v) = self.send(method, url, body).await?;
        if status < 300 {
            return Ok(v);
        }
        let reason = v
            .pointer("/error/root_cause/0/reason")
            .or_else(|| v.pointer("/error/reason"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| match &v {
                Value::String(s) => s.chars().take(300).collect(),
                other => other.to_string().chars().take(300).collect(),
            });
        Err(match status {
            401 | 403 => SqlError::Connect(reason),
            _ => SqlError::Database(reason),
        })
    }

    fn id(body: &Value) -> Result<String, SqlError> {
        match body.get("id") {
            Some(Value::String(s)) if !s.is_empty() => Ok(s.clone()),
            Some(Value::Number(n)) => Ok(n.to_string()),
            _ => Err(SqlError::Rejected("the body needs an `id`".into())),
        }
    }

    pub(crate) async fn read(
        &self,
        op: &str,
        index: &str,
        body: &Value,
        l: Limits,
    ) -> Result<Docs, SqlError> {
        let timeout = vec![("timeout", format!("{}ms", l.timeout_ms))];
        match op {
            "search" => {
                let mut q = match body {
                    Value::Object(o) => o.clone(),
                    _ => Map::new(),
                };
                let limit = u64::from(l.row_limit);
                let asked = q.get("size").and_then(Value::as_u64).unwrap_or(10);
                q.insert("size".into(), json!(asked.min(limit + 1)));
                let v = self
                    .call(
                        Method::POST,
                        self.url(&[index, "_search"], &timeout),
                        Some(&Value::Object(q)),
                    )
                    .await?;
                let hits = v
                    .pointer("/hits/hits")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let truncated = hits.len() as u64 > limit && asked > limit;
                let documents = hits
                    .into_iter()
                    .take(limit as usize)
                    .map(|h| hit(h, index))
                    .collect();
                let mut summary = Map::new();
                if let Some(total) = v
                    .pointer("/hits/total/value")
                    .or_else(|| v.pointer("/hits/total"))
                {
                    summary.insert("total".into(), total.clone());
                }
                if let Some(aggs) = v.get("aggregations") {
                    summary.insert("aggregations".into(), aggs.clone());
                }
                Ok(Docs {
                    documents,
                    truncated,
                    summary: (!summary.is_empty()).then_some(Value::Object(summary)),
                })
            }
            "count" => {
                let v = self
                    .call(Method::POST, self.url(&[index, "_count"], &[]), Some(body))
                    .await?;
                let n = v.get("count").cloned().unwrap_or(Value::Null);
                Ok(Docs {
                    documents: vec![json!({ "count": n })],
                    ..Default::default()
                })
            }
            "get" => {
                let id = Self::id(body)?;
                let (status, v) = self
                    .send(Method::GET, self.url(&[index, "_doc", &id], &[]), None)
                    .await?;
                if status == 404 {
                    return Ok(Docs::default());
                }
                if status >= 300 {
                    return Err(SqlError::Database(
                        v.to_string().chars().take(300).collect(),
                    ));
                }
                Ok(Docs {
                    documents: vec![hit(v, index)],
                    ..Default::default()
                })
            }
            other => Err(SqlError::Rejected(format!("`{other}` isn't a read"))),
        }
    }

    pub(crate) async fn write(
        &self,
        op: &str,
        index: &str,
        body: &Value,
        _l: Limits,
        dry_run: bool,
    ) -> Result<Written, SqlError> {
        // Every write here touches one document.
        let (method, url, payload) = match op {
            "index" => {
                let doc = body
                    .get("document")
                    .filter(|d| d.is_object())
                    .ok_or_else(|| {
                        SqlError::Rejected("the body needs a `document` object".into())
                    })?;
                match body.get("id") {
                    Some(_) => (
                        Method::PUT,
                        self.url(&[index, "_doc", &Self::id(body)?], &[]),
                        Some(doc.clone()),
                    ),
                    None => (
                        Method::POST,
                        self.url(&[index, "_doc"], &[]),
                        Some(doc.clone()),
                    ),
                }
            }
            "update" => {
                let doc = body.get("doc").filter(|d| d.is_object()).ok_or_else(|| {
                    SqlError::Rejected(
                        "the body needs a `doc` object with the fields to change".into(),
                    )
                })?;
                (
                    Method::POST,
                    self.url(&[index, "_update", &Self::id(body)?], &[]),
                    Some(json!({ "doc": doc })),
                )
            }
            "delete" => (
                Method::DELETE,
                self.url(&[index, "_doc", &Self::id(body)?], &[]),
                None,
            ),
            other => return Err(SqlError::Rejected(format!("`{other}` isn't a write"))),
        };
        if dry_run {
            return Ok(Written {
                affected: 1,
                committed: false,
                documents: vec![],
            });
        }
        let v = self.call(method, url, payload.as_ref()).await?;
        let result = v.get("result").and_then(Value::as_str).unwrap_or("");
        let affected = u64::from(!matches!(result, "noop" | "not_found"));
        Ok(Written {
            affected,
            committed: true,
            documents: vec![json!({ "_id": v.get("_id"), "result": result })],
        })
    }

    pub(crate) async fn list_targets(&self) -> Result<Vec<String>, SqlError> {
        let v = self
            .call(
                Method::GET,
                self.url(
                    &["_cat", "indices"],
                    &[("format", "json".into()), ("h", "index".into())],
                ),
                None,
            )
            .await?;
        Ok(v.as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|r| r.get("index").and_then(Value::as_str).map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default())
    }

    pub(crate) async fn describe(&self, index: &str) -> Result<Value, SqlError> {
        let v = self
            .call(Method::GET, self.url(&[index, "_mapping"], &[]), None)
            .await?;
        let mapping = v
            .as_object()
            .and_then(|o| o.values().next())
            .and_then(|m| m.pointer("/mappings/properties"))
            .cloned()
            .unwrap_or(Value::Null);
        Ok(json!({ "index": index, "fields": mapping }))
    }

    pub(crate) async fn probe(&self, allowlist: &[String]) -> Result<ConnectionReport, SqlError> {
        let v = self.call(Method::GET, self.url(&[], &[]), None).await?;
        let mut r =
            ConnectionReport {
                server_version: v.pointer("/version/number").and_then(Value::as_str).map(
                    |n| match v.pointer("/version/distribution").and_then(Value::as_str) {
                        Some("opensearch") => format!("OpenSearch {n}"),
                        _ => format!("Elasticsearch {n}"),
                    },
                ),
                ..Default::default()
            };
        match self
            .send(
                Method::GET,
                self.url(&["_security", "_authenticate"], &[]),
                None,
            )
            .await?
        {
            (200, me) => {
                r.user = me
                    .get("username")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let names: Vec<String> = if allowlist.is_empty() {
                    vec!["*".into()]
                } else {
                    allowlist.to_vec()
                };
                let check = json!({ "index": [{ "names": names, "privileges": ["write", "delete", "index", "create"] }] });
                match self
                    .call(
                        Method::POST,
                        self.url(&["_security", "user", "_has_privileges"], &[]),
                        Some(&check),
                    )
                    .await
                {
                    Ok(p) => {
                        if let Some(idx) = p.get("index").and_then(Value::as_object) {
                            for (name, privs) in idx {
                                if let Some(m) = privs.as_object() {
                                    for (k, ok) in m {
                                        if ok.as_bool() == Some(true) {
                                            r.write_privileges.push(format!("{k} on {name}"));
                                        }
                                    }
                                }
                            }
                        }
                        r.can_write = Some(!r.write_privileges.is_empty());
                    }
                    Err(_) => r
                        .warnings
                        .push("Couldn't check the user's index privileges.".into()),
                }
            }
            _ => {
                r.can_write = Some(true);
                r.write_privileges
                    .push("security is off: anyone who can reach the cluster can write".into());
            }
        }
        Ok(r)
    }
}

/// A search hit or fetched document as `{_id, …_source}`.
fn hit(h: Value, index: &str) -> Value {
    let mut doc = Map::new();
    if let Some(id) = h.get("_id") {
        doc.insert("_id".into(), id.clone());
    }
    if let Some(i) = h.get("_index").and_then(Value::as_str) {
        if i != index {
            doc.insert("_index".into(), json!(i));
        }
    }
    match h.get("_source").or_else(|| h.get("fields")) {
        Some(Value::Object(src)) => doc.extend(src.clone()),
        Some(other) => {
            doc.insert("_source".into(), other.clone());
        }
        None => {}
    }
    Value::Object(doc)
}

fn forge_exec_sql_urldecode(s: &str) -> String {
    crate::pct_decode(s)
}
