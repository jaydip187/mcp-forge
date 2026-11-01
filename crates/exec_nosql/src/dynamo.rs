//! DynamoDB over its JSON API, signed with AWS Signature Version 4 (no AWS
//! SDK). Plain JSON in `Key`, `Item`, `ExpressionAttributeValues` and
//! `ExclusiveStartKey` is turned into DynamoDB attribute values, and items
//! come back as plain JSON. Query and Scan get `Limit` capped at the row
//! limit.

use std::time::Duration;

use forge_exec_sql::{ConnectionReport, SqlError};
use reqwest::Url;
use ring::{digest, hmac};
use serde_json::{json, Map, Value};

use crate::{Docs, Limits, Written};

/// `dynamodb://ACCESS_KEY_ID:SECRET@REGION?endpoint=http://localhost:8000&session_token=…`
#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    endpoint: Url,
    region: String,
    access_key: String,
    secret: String,
    session_token: Option<String>,
}

const TYPES: &[&str] = &["S", "N", "B", "BOOL", "NULL", "M", "L", "SS", "NS", "BS"];

/// A DynamoDB attribute value already (`{"S": "x"}`), with `N` normalized
/// to text.
fn typed(v: &Value) -> Option<Value> {
    let o = v.as_object()?;
    if o.len() != 1 {
        return None;
    }
    let (k, x) = o.iter().next()?;
    if !TYPES.contains(&k.as_str()) {
        return None;
    }
    Some(match (k.as_str(), x) {
        ("N", Value::Number(n)) => json!({ "N": n.to_string() }),
        _ => v.clone(),
    })
}

/// Plain JSON → attribute value (typed values pass through).
pub(crate) fn marshal(v: &Value) -> Value {
    if let Some(t) = typed(v) {
        return t;
    }
    match v {
        Value::Null => json!({ "NULL": true }),
        Value::Bool(b) => json!({ "BOOL": b }),
        Value::Number(n) => json!({ "N": n.to_string() }),
        Value::String(s) => json!({ "S": s }),
        Value::Array(a) => json!({ "L": a.iter().map(marshal).collect::<Vec<_>>() }),
        Value::Object(o) => json!({ "M": marshal_map(o) }),
    }
}

fn marshal_map(o: &Map<String, Value>) -> Map<String, Value> {
    o.iter().map(|(k, v)| (k.clone(), marshal(v))).collect()
}

/// Attribute value → plain JSON.
pub(crate) fn unmarshal(v: &Value) -> Value {
    let Some((k, x)) = v.as_object().and_then(|o| o.iter().next()) else {
        return v.clone();
    };
    match k.as_str() {
        "S" | "B" => x.clone(),
        "N" => x
            .as_str()
            .and_then(|s| {
                s.parse::<i64>()
                    .ok()
                    .map(Value::from)
                    .or_else(|| s.parse::<f64>().ok().map(Value::from))
            })
            .unwrap_or_else(|| x.clone()),
        "BOOL" => x.clone(),
        "NULL" => Value::Null,
        "M" => Value::Object(x.as_object().map(unmarshal_map).unwrap_or_default()),
        "L" => Value::Array(
            x.as_array()
                .map(|a| a.iter().map(unmarshal).collect())
                .unwrap_or_default(),
        ),
        "SS" | "BS" => x.clone(),
        "NS" => Value::Array(
            x.as_array()
                .map(|a| a.iter().map(|n| unmarshal(&json!({ "N": n }))).collect())
                .unwrap_or_default(),
        ),
        _ => v.clone(),
    }
}

fn unmarshal_map(o: &Map<String, Value>) -> Map<String, Value> {
    o.iter().map(|(k, v)| (k.clone(), unmarshal(v))).collect()
}

/// The request with `TableName` set and plain values marshalled.
pub(crate) fn prepare(table: &str, body: &Value) -> Map<String, Value> {
    let mut req = match body {
        Value::Object(o) => o.clone(),
        _ => Map::new(),
    };
    req.insert("TableName".into(), json!(table));
    for key in ["Key", "Item", "ExclusiveStartKey"] {
        if let Some(Value::Object(o)) = req.get(key) {
            let m = marshal_map(o);
            req.insert(key.into(), Value::Object(m));
        }
    }
    if let Some(Value::Object(o)) = req.get("ExpressionAttributeValues") {
        let m = marshal_map(o);
        req.insert("ExpressionAttributeValues".into(), Value::Object(m));
    }
    req
}

fn sha256_hex(data: &[u8]) -> String {
    hex::encode(digest::digest(&digest::SHA256, data))
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, key), data)
        .as_ref()
        .to_vec()
}

/// SigV4 signing key for a date (`YYYYMMDD`), region and service.
pub(crate) fn signing_key(secret: &str, date: &str, region: &str, service: &str) -> Vec<u8> {
    let k = hmac_sha256(format!("AWS4{secret}").as_bytes(), date.as_bytes());
    let k = hmac_sha256(&k, region.as_bytes());
    let k = hmac_sha256(&k, service.as_bytes());
    hmac_sha256(&k, b"aws4_request")
}

/// AWS credentials for signing.
pub(crate) struct Creds<'a> {
    pub region: &'a str,
    pub access_key: &'a str,
    pub secret: &'a str,
    pub session_token: Option<&'a str>,
}

/// Headers to send, including `Authorization`, for a DynamoDB POST.
pub(crate) fn sign(
    creds: &Creds<'_>,
    host: &str,
    target: &str,
    payload: &[u8],
    amz_date: &str,
) -> Vec<(String, String)> {
    let Creds {
        region,
        access_key,
        secret,
        session_token,
    } = *creds;
    let date = &amz_date[..8];
    let mut headers: Vec<(String, String)> = vec![
        ("content-type".into(), "application/x-amz-json-1.0".into()),
        ("host".into(), host.into()),
        ("x-amz-date".into(), amz_date.into()),
        ("x-amz-target".into(), target.into()),
    ];
    if let Some(t) = session_token {
        headers.push(("x-amz-security-token".into(), t.into()));
    }
    headers.sort();
    let canonical_headers: String = headers
        .iter()
        .map(|(k, v)| format!("{k}:{}\n", v.trim()))
        .collect();
    let signed: Vec<&str> = headers.iter().map(|(k, _)| k.as_str()).collect();
    let signed = signed.join(";");
    let canonical = format!(
        "POST\n/\n\n{canonical_headers}\n{signed}\n{}",
        sha256_hex(payload)
    );
    let scope = format!("{date}/{region}/dynamodb/aws4_request");
    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        sha256_hex(canonical.as_bytes())
    );
    let sig = hex::encode(hmac_sha256(
        &signing_key(secret, date, region, "dynamodb"),
        to_sign.as_bytes(),
    ));
    headers.push((
        "authorization".into(),
        format!("AWS4-HMAC-SHA256 Credential={access_key}/{scope}, SignedHeaders={signed}, Signature={sig}"),
    ));
    headers
}

impl Client {
    pub(crate) fn new(dsn: &str, timeout_ms: u32) -> Result<Self, SqlError> {
        let url = Url::parse(dsn.trim())
            .map_err(|e| SqlError::Config(format!("invalid connection string: {e}")))?;
        let shape = || {
            SqlError::Config(
                "the connection string must look like dynamodb://ACCESS_KEY_ID:SECRET@us-east-1 \
                 (percent-encode / and + in the secret)"
                    .into(),
            )
        };
        if url.scheme() != "dynamodb" || url.username().is_empty() || url.password().is_none() {
            return Err(shape());
        }
        let region = url.host_str().ok_or_else(shape)?.to_owned();
        let mut endpoint = None;
        let mut session_token = None;
        for (k, v) in url.query_pairs() {
            match k.as_ref() {
                "endpoint" => endpoint = Some(v.into_owned()),
                "session_token" => session_token = Some(v.into_owned()),
                _ => {}
            }
        }
        let endpoint = match endpoint {
            Some(e) => {
                Url::parse(&e).map_err(|e| SqlError::Config(format!("invalid endpoint: {e}")))?
            }
            None => Url::parse(&format!("https://dynamodb.{region}.amazonaws.com/"))
                .map_err(|_| shape())?,
        };
        let http = reqwest::Client::builder()
            .connect_timeout(
                Duration::from_millis(u64::from(timeout_ms)).max(Duration::from_secs(5)),
            )
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| SqlError::Config(format!("HTTP client: {e}")))?;
        Ok(Self {
            http,
            endpoint,
            region,
            access_key: crate::pct_decode(url.username()),
            secret: crate::pct_decode(url.password().unwrap_or_default()),
            session_token,
        })
    }

    async fn call(&self, action: &str, req: &Value) -> Result<Value, SqlError> {
        let payload = req.to_string();
        let host = match self.endpoint.port() {
            Some(p) => format!("{}:{p}", self.endpoint.host_str().unwrap_or_default()),
            None => self.endpoint.host_str().unwrap_or_default().to_owned(),
        };
        let amz_date = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
        let creds = Creds {
            region: &self.region,
            access_key: &self.access_key,
            secret: &self.secret,
            session_token: self.session_token.as_deref(),
        };
        let headers = sign(
            &creds,
            &host,
            &format!("DynamoDB_20120810.{action}"),
            payload.as_bytes(),
            &amz_date,
        );
        let mut r = self.http.post(self.endpoint.clone()).body(payload);
        for (k, v) in headers.into_iter().filter(|(k, _)| k != "host") {
            r = r.header(k, v);
        }
        let resp = r
            .send()
            .await
            .map_err(|e| SqlError::Connect(e.without_url().to_string()))?;
        let status = resp.status().as_u16();
        let text = resp
            .text()
            .await
            .map_err(|e| SqlError::Connect(e.without_url().to_string()))?;
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::String(text));
        if status < 300 {
            return Ok(v);
        }
        let kind = v
            .get("__type")
            .and_then(Value::as_str)
            .and_then(|t| t.rsplit('#').next())
            .unwrap_or("Error")
            .to_owned();
        let msg = v
            .get("message")
            .or_else(|| v.get("Message"))
            .and_then(Value::as_str)
            .unwrap_or("request failed");
        let text = format!("{kind}: {msg}");
        Err(match kind.as_str() {
            "UnrecognizedClientException"
            | "AccessDeniedException"
            | "InvalidSignatureException"
            | "MissingAuthenticationTokenException" => SqlError::Connect(text),
            _ => SqlError::Database(text),
        })
    }

    pub(crate) async fn read(
        &self,
        op: &str,
        table: &str,
        body: &Value,
        l: Limits,
    ) -> Result<Docs, SqlError> {
        let mut req = prepare(table, body);
        let limit = u64::from(l.row_limit);
        match op {
            "GetItem" => {
                let v = self.call(op, &Value::Object(req)).await?;
                Ok(Docs {
                    documents: v
                        .get("Item")
                        .map(|i| vec![unmarshal(&json!({ "M": i }))])
                        .unwrap_or_default(),
                    ..Default::default()
                })
            }
            "Query" | "Scan" => {
                let asked = req.get("Limit").and_then(Value::as_u64);
                let capped = asked.map_or(limit + 1, |a| a.min(limit + 1));
                req.insert("Limit".into(), json!(capped));
                let v = self.call(op, &Value::Object(req)).await?;
                let items = v
                    .get("Items")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let more = v.get("LastEvaluatedKey").is_some();
                let truncated =
                    items.len() as u64 > limit || (more && asked.is_none_or(|a| a > limit));
                let documents = items
                    .iter()
                    .take(limit as usize)
                    .map(|i| unmarshal(&json!({ "M": i })))
                    .collect();
                let mut summary = Map::new();
                if let Some(k) = v.get("LastEvaluatedKey") {
                    summary.insert("last_evaluated_key".into(), unmarshal(&json!({ "M": k })));
                }
                Ok(Docs {
                    documents,
                    truncated,
                    summary: (!summary.is_empty()).then_some(Value::Object(summary)),
                })
            }
            other => Err(SqlError::Rejected(format!("`{other}` isn't a read"))),
        }
    }

    pub(crate) async fn write(
        &self,
        op: &str,
        table: &str,
        body: &Value,
        _l: Limits,
        dry_run: bool,
    ) -> Result<Written, SqlError> {
        if !matches!(op, "PutItem" | "UpdateItem" | "DeleteItem") {
            return Err(SqlError::Rejected(format!("`{op}` isn't a write")));
        }
        // One item per call.
        if dry_run {
            return Ok(Written {
                affected: 1,
                committed: false,
                documents: vec![],
            });
        }
        let v = self.call(op, &Value::Object(prepare(table, body))).await?;
        let documents = v
            .get("Attributes")
            .map(|a| vec![unmarshal(&json!({ "M": a }))])
            .unwrap_or_default();
        Ok(Written {
            affected: 1,
            committed: true,
            documents,
        })
    }

    pub(crate) async fn list_targets(&self) -> Result<Vec<String>, SqlError> {
        let mut names = Vec::new();
        let mut start: Option<String> = None;
        for _ in 0..20 {
            let mut req = json!({ "Limit": 100 });
            if let Some(s) = &start {
                req["ExclusiveStartTableName"] = json!(s);
            }
            let v = self.call("ListTables", &req).await?;
            if let Some(a) = v.get("TableNames").and_then(Value::as_array) {
                names.extend(a.iter().filter_map(Value::as_str).map(str::to_owned));
            }
            match v.get("LastEvaluatedTableName").and_then(Value::as_str) {
                Some(s) => start = Some(s.to_owned()),
                None => break,
            }
        }
        Ok(names)
    }

    pub(crate) async fn describe(&self, table: &str) -> Result<Value, SqlError> {
        let v = self
            .call("DescribeTable", &json!({ "TableName": table }))
            .await?;
        let t = v.get("Table").cloned().unwrap_or(Value::Null);
        let indexes = |k: &str| {
            t.get(k).and_then(Value::as_array).map(|a| {
                a.iter()
                    .map(|i| json!({ "name": i.get("IndexName"), "key": i.get("KeySchema") }))
                    .collect::<Vec<_>>()
            })
        };
        Ok(json!({
            "table": table,
            "key": t.get("KeySchema"),
            "attributes": t.get("AttributeDefinitions"),
            "global_indexes": indexes("GlobalSecondaryIndexes"),
            "local_indexes": indexes("LocalSecondaryIndexes"),
            "item_count": t.get("ItemCount"),
            "status": t.get("TableStatus"),
        }))
    }

    pub(crate) async fn probe(&self) -> Result<ConnectionReport, SqlError> {
        self.call("ListTables", &json!({ "Limit": 1 })).await?;
        Ok(ConnectionReport {
            server_version: Some(format!("DynamoDB ({})", self.region)),
            user: Some(self.access_key.clone()),
            can_write: None,
            warnings: vec![
                "DynamoDB permissions come from IAM: give these keys read-only actions (GetItem, Query, \
                 Scan, DescribeTable, ListTables) unless saved writes are on."
                    .into(),
            ],
            ..Default::default()
        })
    }
}
