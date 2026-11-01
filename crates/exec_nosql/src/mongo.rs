//! MongoDB through the official driver. Reads get a server-side
//! `maxTimeMS` and are capped at the row limit; `update_*`/`delete_*` count
//! the matching documents first, so the write cap and dry runs work without
//! transactions. Bodies are JSON (MongoDB extended JSON such as
//! `{"$oid": …}` works); results come back as plain JSON.

use std::time::Duration;

use forge_exec_sql::{ConnectionReport, SqlError};
use futures_util::TryStreamExt;
use mongodb::bson::{doc, Bson, Document};
use mongodb::options::ClientOptions;
use mongodb::{Collection, Database};
use serde_json::{json, Map, Value};

use crate::{Docs, Limits, Written};

#[derive(Clone)]
pub struct Db {
    db: Database,
}

fn db_err(e: mongodb::error::Error) -> SqlError {
    use mongodb::error::ErrorKind;
    match *e.kind {
        ErrorKind::ServerSelection { .. } | ErrorKind::Io(_) | ErrorKind::DnsResolve { .. } => {
            SqlError::Connect(e.to_string())
        }
        ErrorKind::Authentication { .. } => SqlError::Connect(e.to_string()),
        // MaxTimeMSExpired.
        ErrorKind::Command(ref c) if c.code == 50 => SqlError::Timeout(0),
        ErrorKind::Command(ref c) => SqlError::Database(c.message.clone()),
        _ => SqlError::Database(e.to_string()),
    }
}

/// JSON (extended JSON allowed) → BSON document.
fn to_doc(v: &Value, what: &str) -> Result<Document, SqlError> {
    match v {
        Value::Null => Ok(Document::new()),
        Value::Object(_) => match Bson::try_from(v.clone()) {
            Ok(Bson::Document(d)) => Ok(d),
            Ok(_) | Err(_) => Err(SqlError::Rejected(format!(
                "`{what}` isn't a valid document"
            ))),
        },
        _ => Err(SqlError::Rejected(format!("`{what}` must be an object"))),
    }
}

fn field<'a>(body: &'a Value, name: &str) -> &'a Value {
    body.get(name).unwrap_or(&Value::Null)
}

/// BSON → plain JSON: ObjectIds and decimals as text, dates as ISO text.
pub(crate) fn to_json(b: Bson) -> Value {
    match b {
        Bson::Double(f) => {
            serde_json::Number::from_f64(f).map_or(Value::String(f.to_string()), Value::Number)
        }
        Bson::String(s) | Bson::Symbol(s) | Bson::JavaScriptCode(s) => Value::String(s),
        Bson::Array(a) => Value::Array(a.into_iter().map(to_json).collect()),
        Bson::Document(d) => Value::Object(d.into_iter().map(|(k, v)| (k, to_json(v))).collect()),
        Bson::Boolean(b) => Value::Bool(b),
        Bson::Null | Bson::Undefined => Value::Null,
        Bson::Int32(i) => json!(i),
        Bson::Int64(i) => json!(i),
        Bson::ObjectId(o) => Value::String(o.to_hex()),
        Bson::DateTime(d) => d
            .try_to_rfc3339_string()
            .map_or_else(|_| json!(d.timestamp_millis()), Value::String),
        Bson::Decimal128(d) => Value::String(d.to_string()),
        Bson::Binary(b) => Value::String(format!("<binary, {} bytes>", b.bytes.len())),
        other => other.into_relaxed_extjson(),
    }
}

impl Db {
    /// `mongodb://user:password@host:27017/database` (or `mongodb+srv://`).
    pub(crate) async fn connect(dsn: &str, timeout_ms: u32) -> Result<Self, SqlError> {
        let mut opts = ClientOptions::parse(dsn.trim())
            .await
            .map_err(|e| SqlError::Config(format!("invalid connection string: {e}")))?;
        opts.app_name = Some("mcp-forge".into());
        let t = Duration::from_millis(u64::from(timeout_ms)).max(Duration::from_secs(5));
        opts.server_selection_timeout = Some(t);
        opts.connect_timeout = Some(t);
        let client = mongodb::Client::with_options(opts).map_err(db_err)?;
        let db = client.default_database().ok_or_else(|| {
            SqlError::Config(
                "put the database name in the connection string: mongodb://…/database".into(),
            )
        })?;
        Ok(Self { db })
    }

    fn coll(&self, name: &str) -> Collection<Document> {
        self.db.collection::<Document>(name)
    }

    pub(crate) async fn read(
        &self,
        op: &str,
        target: &str,
        body: &Value,
        l: Limits,
    ) -> Result<Docs, SqlError> {
        let coll = self.coll(target);
        let max_time = Duration::from_millis(u64::from(l.timeout_ms));
        let limit = u64::from(l.row_limit);
        let filter = to_doc(field(body, "filter"), "filter")?;
        match op {
            "find" => {
                let asked = field(body, "limit").as_u64();
                let fetch = asked.map_or(limit + 1, |a| a.min(limit + 1));
                let mut find = coll.find(filter).limit(fetch as i64).max_time(max_time);
                if !field(body, "projection").is_null() {
                    find = find.projection(to_doc(field(body, "projection"), "projection")?);
                }
                if !field(body, "sort").is_null() {
                    find = find.sort(to_doc(field(body, "sort"), "sort")?);
                }
                if let Some(skip) = field(body, "skip").as_u64() {
                    find = find.skip(skip);
                }
                let docs: Vec<Document> = find
                    .await
                    .map_err(db_err)?
                    .try_collect()
                    .await
                    .map_err(db_err)?;
                let truncated = docs.len() as u64 > limit;
                Ok(Docs {
                    documents: docs
                        .into_iter()
                        .take(limit as usize)
                        .map(|d| to_json(Bson::Document(d)))
                        .collect(),
                    truncated,
                    summary: None,
                })
            }
            "find_one" => {
                let mut q = coll.find_one(filter).max_time(max_time);
                if !field(body, "projection").is_null() {
                    q = q.projection(to_doc(field(body, "projection"), "projection")?);
                }
                let d = q.await.map_err(db_err)?;
                Ok(Docs {
                    documents: d
                        .map(|d| vec![to_json(Bson::Document(d))])
                        .unwrap_or_default(),
                    ..Default::default()
                })
            }
            "aggregate" => {
                let stages = field(body, "pipeline").as_array().ok_or_else(|| {
                    SqlError::Rejected("`pipeline` must be an array of stages".into())
                })?;
                let mut pipeline: Vec<Document> = stages
                    .iter()
                    .map(|s| to_doc(s, "pipeline stage"))
                    .collect::<Result<_, _>>()?;
                pipeline.push(doc! { "$limit": (limit + 1) as i64 });
                let docs: Vec<Document> = coll
                    .aggregate(pipeline)
                    .max_time(max_time)
                    .await
                    .map_err(db_err)?
                    .try_collect()
                    .await
                    .map_err(db_err)?;
                let truncated = docs.len() as u64 > limit;
                Ok(Docs {
                    documents: docs
                        .into_iter()
                        .take(limit as usize)
                        .map(|d| to_json(Bson::Document(d)))
                        .collect(),
                    truncated,
                    summary: None,
                })
            }
            "count" => {
                let n = coll
                    .count_documents(filter)
                    .max_time(max_time)
                    .await
                    .map_err(db_err)?;
                Ok(Docs {
                    documents: vec![json!({ "count": n })],
                    ..Default::default()
                })
            }
            "distinct" => {
                let name = field(body, "field")
                    .as_str()
                    .ok_or_else(|| SqlError::Rejected("`field` names the field to list".into()))?;
                let values = coll
                    .distinct(name, filter)
                    .max_time(max_time)
                    .await
                    .map_err(db_err)?;
                let truncated = values.len() as u64 > limit;
                Ok(Docs {
                    documents: values
                        .into_iter()
                        .take(limit as usize)
                        .map(to_json)
                        .collect(),
                    truncated,
                    summary: None,
                })
            }
            other => Err(SqlError::Rejected(format!("`{other}` isn't a read"))),
        }
    }

    pub(crate) async fn write(
        &self,
        op: &str,
        target: &str,
        body: &Value,
        l: Limits,
        dry_run: bool,
    ) -> Result<Written, SqlError> {
        let coll = self.coll(target);
        let max = u64::from(l.max_affected);
        let max_time = Duration::from_millis(u64::from(l.timeout_ms));
        if op == "insert_one" {
            let doc = to_doc(field(body, "document"), "document")?;
            if doc.is_empty() {
                return Err(SqlError::Rejected(
                    "`document` is the document to insert".into(),
                ));
            }
            if dry_run {
                return Ok(Written {
                    affected: 1,
                    committed: false,
                    documents: vec![],
                });
            }
            let r = coll.insert_one(doc).await.map_err(db_err)?;
            return Ok(Written {
                affected: 1,
                committed: true,
                documents: vec![json!({ "_id": to_json(r.inserted_id) })],
            });
        }
        let filter = to_doc(field(body, "filter"), "filter")?;
        // Count first: the cap and dry runs need the number before changing anything.
        let many = op.ends_with("_many");
        let matching = coll
            .count_documents(filter.clone())
            .limit(if many { max + 1 } else { 1 })
            .max_time(max_time)
            .await
            .map_err(db_err)?;
        if matching > max {
            return Err(SqlError::TooManyRows {
                affected: matching,
                max: l.max_affected,
                at_least: true,
            });
        }
        if dry_run {
            return Ok(Written {
                affected: matching,
                committed: false,
                documents: vec![],
            });
        }
        let affected = match op {
            "update_one" | "update_many" => {
                let update = field(body, "update");
                let upsert = field(body, "upsert").as_bool().unwrap_or(false);
                let r = match update {
                    Value::Array(stages) => {
                        let p: Vec<Document> = stages
                            .iter()
                            .map(|s| to_doc(s, "update stage"))
                            .collect::<Result<_, _>>()?;
                        if many {
                            coll.update_many(filter, p).upsert(upsert).await
                        } else {
                            coll.update_one(filter, p).upsert(upsert).await
                        }
                    }
                    other => {
                        let u = to_doc(other, "update")?;
                        if u.is_empty() {
                            return Err(SqlError::Rejected(
                                "`update` says what to change, e.g. {\"$set\": {…}}".into(),
                            ));
                        }
                        if many {
                            coll.update_many(filter, u).upsert(upsert).await
                        } else {
                            coll.update_one(filter, u).upsert(upsert).await
                        }
                    }
                }
                .map_err(db_err)?;
                r.matched_count + u64::from(r.upserted_id.is_some())
            }
            "delete_one" => coll.delete_one(filter).await.map_err(db_err)?.deleted_count,
            "delete_many" => {
                coll.delete_many(filter)
                    .await
                    .map_err(db_err)?
                    .deleted_count
            }
            other => return Err(SqlError::Rejected(format!("`{other}` isn't a write"))),
        };
        Ok(Written {
            affected,
            committed: true,
            documents: vec![],
        })
    }

    pub(crate) async fn list_targets(&self) -> Result<Vec<String>, SqlError> {
        self.db.list_collection_names().await.map_err(db_err)
    }

    /// Field names and types seen in a sample, plus index names.
    pub(crate) async fn describe(&self, target: &str) -> Result<Value, SqlError> {
        let coll = self.coll(target);
        let sample: Vec<Document> = coll
            .find(Document::new())
            .limit(20)
            .max_time(Duration::from_secs(10))
            .await
            .map_err(db_err)?
            .try_collect()
            .await
            .map_err(db_err)?;
        let mut fields: Map<String, Value> = Map::new();
        for d in &sample {
            for (k, v) in d {
                let ty = format!("{:?}", v.element_type()).to_lowercase();
                let entry = fields.entry(k.clone()).or_insert_with(|| json!([]));
                if let Some(a) = entry.as_array_mut() {
                    if !a.iter().any(|x| x == &json!(ty)) {
                        a.push(json!(ty));
                    }
                }
            }
        }
        let indexes = coll.list_index_names().await.unwrap_or_default();
        Ok(json!({
            "collection": target,
            "fields": fields,
            "indexes": indexes,
            "sampled_documents": sample.len(),
        }))
    }

    pub(crate) async fn probe(&self) -> Result<ConnectionReport, SqlError> {
        let info = self
            .db
            .run_command(doc! { "buildInfo": 1 })
            .await
            .map_err(db_err)?;
        let status = self
            .db
            .run_command(doc! { "connectionStatus": 1, "showPrivileges": true })
            .await
            .map_err(db_err)?;
        let status = to_json(Bson::Document(status));
        let mut r = ConnectionReport {
            server_version: info.get_str("version").ok().map(|v| format!("MongoDB {v}")),
            user: status
                .pointer("/authInfo/authenticatedUsers/0/user")
                .and_then(Value::as_str)
                .map(str::to_owned),
            ..Default::default()
        };
        const WRITE: &[&str] = &[
            "insert",
            "update",
            "remove",
            "createCollection",
            "dropCollection",
            "createIndex",
            "dropDatabase",
            "renameCollectionSameDB",
        ];
        match status
            .pointer("/authInfo/authenticatedUserPrivileges")
            .and_then(Value::as_array)
        {
            Some(privs) if r.user.is_some() => {
                for p in privs {
                    let actions = p
                        .get("actions")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    let on = p.get("resource").map(|x| x.to_string()).unwrap_or_default();
                    for a in actions
                        .iter()
                        .filter_map(Value::as_str)
                        .filter(|a| WRITE.contains(a))
                    {
                        let line = format!("{a} on {on}");
                        if !r.write_privileges.contains(&line) {
                            r.write_privileges.push(line);
                        }
                    }
                }
                r.can_write = Some(!r.write_privileges.is_empty());
            }
            _ => {
                r.can_write = Some(true);
                r.write_privileges
                    .push("no authentication: anyone who can reach the server can write".into());
            }
        }
        Ok(r)
    }
}
