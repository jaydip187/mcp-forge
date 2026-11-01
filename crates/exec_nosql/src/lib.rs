//! NoSQL connections (MongoDB, Redis, DynamoDB, Elasticsearch) and the
//! executor for NoSQL tools. Reads are the default; writes need the
//! connection's access mode, are capped by its row limit for changes, can
//! be dry-run, and are audited like SQL writes.
//!
//! Layers, each on its own:
//! 1. operations come from a fixed list per engine, classified read/write;
//! 2. targets must be allowed (system names never are);
//! 3. bodies pass per-engine rules (no code execution, no writing elsewhere,
//!    no reading collections that aren't allowed);
//! 4. arguments fill `{{param}}` templates as JSON values, never as keys;
//! 5. results are capped by row limit, time limit and cell length, and
//!    masked fields are replaced.

mod dynamo;
mod elastic;
mod mongo;
mod outcome;
pub mod policy;
mod redis_db;

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use forge_core::{
    check_args, AccessMode, ArgBinding, ArgTarget, AuditOutcome, DbConnection, DerivedInput,
    Driver, NosqlSpec, OpClass, SecretResolver, DESCRIBE_TARGET, LIST_TARGETS, NOSQL_BODY_ARG,
    NOSQL_OPERATION_ARG, NOSQL_PATTERN_ARG, NOSQL_TARGET_ARG, RUN_QUERY,
};
use forge_exec_sql::{
    AuditDraft, ConnectionReport, Redactor, SqlError, StatementClass, WriteInfo, DRY_RUN_ARG,
};
use serde_json::{json, Map, Value};
use uuid::Uuid;

pub use outcome::{NosqlOutcome, NosqlRun};
pub use policy::check_tool;

/// Text cells longer than this are cut (like SQL cells).
const MAX_CELL_CHARS: usize = 1_000;
/// Most names `list_targets` returns.
const MAX_TARGETS: usize = 2_000;

/// What one engine call read.
#[derive(Debug, Default)]
pub(crate) struct Docs {
    pub documents: Vec<Value>,
    pub truncated: bool,
    /// Counts, totals, aggregations.
    pub summary: Option<Value>,
}

/// What one write did (or, on a dry run, would do).
#[derive(Debug, Default)]
pub(crate) struct Written {
    pub affected: u64,
    pub committed: bool,
    pub documents: Vec<Value>,
}

/// Request details shared by the engines.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Limits {
    pub row_limit: u32,
    pub timeout_ms: u32,
    pub max_affected: u32,
}

/// A connected client, one per connection.
#[derive(Clone)]
pub(crate) enum Client {
    Mongo(mongo::Db),
    Redis(redis_db::Conn),
    Dynamo(dynamo::Client),
    Elastic(elastic::Client),
}

impl Client {
    async fn connect(driver: Driver, dsn: &str, timeout_ms: u32) -> Result<Self, SqlError> {
        Ok(match driver {
            Driver::Mongodb => Client::Mongo(mongo::Db::connect(dsn, timeout_ms).await?),
            Driver::Redis => Client::Redis(redis_db::Conn::connect(dsn, timeout_ms).await?),
            Driver::Dynamodb => Client::Dynamo(dynamo::Client::new(dsn, timeout_ms)?),
            Driver::Elasticsearch => Client::Elastic(elastic::Client::new(dsn, timeout_ms)?),
            other => {
                return Err(SqlError::Config(format!(
                    "{} connections run SQL tools, not NoSQL tools",
                    other.as_str()
                )))
            }
        })
    }

    async fn read(
        &self,
        op: &str,
        target: &str,
        body: &Value,
        l: Limits,
    ) -> Result<Docs, SqlError> {
        match self {
            Client::Mongo(c) => c.read(op, target, body, l).await,
            Client::Redis(c) => c.read(op, target, body, l).await,
            Client::Dynamo(c) => c.read(op, target, body, l).await,
            Client::Elastic(c) => c.read(op, target, body, l).await,
        }
    }

    async fn write(
        &self,
        op: &str,
        target: &str,
        body: &Value,
        l: Limits,
        dry_run: bool,
    ) -> Result<Written, SqlError> {
        match self {
            Client::Mongo(c) => c.write(op, target, body, l, dry_run).await,
            Client::Redis(c) => c.write(op, target, body, l, dry_run).await,
            Client::Dynamo(c) => c.write(op, target, body, l, dry_run).await,
            Client::Elastic(c) => c.write(op, target, body, l, dry_run).await,
        }
    }

    async fn list_targets(&self, pattern: Option<&str>) -> Result<Vec<String>, SqlError> {
        match self {
            Client::Mongo(c) => c.list_targets().await,
            Client::Redis(c) => c.list_targets(pattern, MAX_TARGETS).await,
            Client::Dynamo(c) => c.list_targets().await,
            Client::Elastic(c) => c.list_targets().await,
        }
    }

    async fn describe(&self, target: &str) -> Result<Value, SqlError> {
        match self {
            Client::Mongo(c) => c.describe(target).await,
            Client::Redis(c) => c.describe(target).await,
            Client::Dynamo(c) => c.describe(target).await,
            Client::Elastic(c) => c.describe(target).await,
        }
    }

    async fn probe(&self, allowlist: &[String]) -> Result<ConnectionReport, SqlError> {
        match self {
            Client::Mongo(c) => c.probe().await,
            Client::Redis(c) => c.probe().await,
            Client::Dynamo(c) => c.probe().await,
            Client::Elastic(c) => c.probe(allowlist).await,
        }
    }
}

/// Percent-decodes URL user info (`+` stays `+`).
pub(crate) fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            if let Some(v) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_owned())
}

/// Caps long strings anywhere in a document (objects stay objects).
pub(crate) fn cap(v: Value) -> Value {
    match v {
        Value::String(s) if s.chars().count() > MAX_CELL_CHARS => {
            let n = s.chars().count();
            let mut out: String = s.chars().take(MAX_CELL_CHARS).collect();
            out.push_str(&format!("… [{} more chars]", n - MAX_CELL_CHARS));
            Value::String(out)
        }
        Value::Array(a) => Value::Array(a.into_iter().map(cap).collect()),
        Value::Object(o) => Value::Object(o.into_iter().map(|(k, x)| (k, cap(x))).collect()),
        other => other,
    }
}

/// Whether a tool changes data through this connection.
pub fn tool_writes(spec: &NosqlSpec, conn: &DbConnection) -> bool {
    match spec.operation.as_str() {
        RUN_QUERY => conn.access_mode == AccessMode::FullAccess,
        LIST_TARGETS | DESCRIBE_TARGET => false,
        _ => spec.writes(conn.driver),
    }
}

/// The tool's MCP input: its own arguments, plus `dry_run` when it writes.
pub fn tool_input(spec: &NosqlSpec, conn: &DbConnection) -> DerivedInput {
    let mut d = spec.derive_input();
    if tool_writes(spec, conn) {
        if let Some(props) = d
            .schema
            .get_mut("properties")
            .and_then(Value::as_object_mut)
        {
            props.insert(
                DRY_RUN_ARG.into(),
                json!({ "type": "boolean",
                    "description": "If true, report how many items it would change without changing them." }),
            );
        }
        d.bindings.push(ArgBinding {
            arg: DRY_RUN_ARG.into(),
            target: ArgTarget::SqlParam {
                name: DRY_RUN_ARG.into(),
            },
            required: false,
        });
    }
    d
}

/// Cached clients: connection → (settings fingerprint, client).
type ClientMap = HashMap<Uuid, (u64, Client)>;

/// Runs NoSQL tools; keeps one client per connection, rebuilt when the
/// connection string or limits change.
#[derive(Clone)]
pub struct NosqlExecutor {
    secrets: Arc<dyn SecretResolver>,
    clients: Arc<Mutex<ClientMap>>,
}

impl NosqlExecutor {
    pub fn new(secrets: Arc<dyn SecretResolver>) -> Self {
        Self {
            secrets,
            clients: Arc::default(),
        }
    }

    async fn resolve_dsn(&self, conn: &DbConnection) -> Result<String, SqlError> {
        let secrets = self.secrets.clone();
        let r = conn.dsn_ref.clone();
        tokio::task::spawn_blocking(move || secrets.resolve(&r))
            .await
            .map_err(|e| SqlError::Config(format!("secret lookup failed: {e}")))?
            .map_err(|e| SqlError::Config(format!("connection string unavailable: {e}")))
    }

    async fn client(&self, conn: &DbConnection, dsn: &str) -> Result<Client, SqlError> {
        let mut h = DefaultHasher::new();
        (conn.driver, dsn, conn.timeout_ms).hash(&mut h);
        let key = h.finish();
        if let Some((k, c)) = self
            .clients
            .lock()
            .expect("client map poisoned")
            .get(&conn.id)
        {
            if *k == key {
                return Ok(c.clone());
            }
        }
        let c = Client::connect(conn.driver, dsn, conn.timeout_ms).await?;
        self.clients
            .lock()
            .expect("client map poisoned")
            .insert(conn.id, (key, c.clone()));
        Ok(c)
    }

    /// Drops the cached client of a deleted or edited connection.
    pub fn forget(&self, connection_id: Uuid) {
        self.clients
            .lock()
            .expect("client map poisoned")
            .remove(&connection_id);
    }

    /// Server version, user and whether that user may write.
    pub async fn test_connection(&self, conn: &DbConnection) -> ConnectionReport {
        let started = Instant::now();
        let dsn = match self.resolve_dsn(conn).await {
            Ok(d) => d,
            Err(e) => {
                return ConnectionReport {
                    error: Some(e.to_string()),
                    ..Default::default()
                }
            }
        };
        let redactor = Redactor::new(&dsn);
        let budget =
            Duration::from_millis(u64::from(conn.timeout_ms.max(5_000))) + Duration::from_secs(2);
        let r = tokio::time::timeout(budget, async {
            let c = self.client(conn, &dsn).await?;
            c.probe(&conn.table_allowlist).await
        })
        .await
        .unwrap_or(Err(SqlError::Timeout(conn.timeout_ms)));
        let mut report = match r {
            Ok(mut rep) => {
                rep.ok = true;
                rep
            }
            Err(e) => ConnectionReport {
                error: Some(e.redact(&redactor).to_string()),
                ..Default::default()
            },
        };
        report.elapsed_ms = started.elapsed().as_millis() as u64;
        report
    }

    /// Runs a NoSQL tool with the agent's arguments. Writes come back with
    /// an audit record, even when they fail.
    pub async fn run(
        &self,
        conn: &DbConnection,
        spec: &NosqlSpec,
        args: &Map<String, Value>,
    ) -> NosqlRun {
        let started = Instant::now();
        let mut run = match self.dispatch(conn, spec, args).await {
            Ok(r) => r,
            Err(e) => NosqlRun {
                result: Err(e),
                audit: None,
            },
        };
        if let Ok(o) = &mut run.result {
            o.elapsed_ms = started.elapsed().as_millis() as u64;
        }
        run
    }

    async fn dispatch(
        &self,
        conn: &DbConnection,
        spec: &NosqlSpec,
        args: &Map<String, Value>,
    ) -> Result<NosqlRun, SqlError> {
        let driver = conn.driver;
        if driver.is_sql() {
            return Err(SqlError::Config(
                "this tool's connection runs SQL tools, not NoSQL tools".into(),
            ));
        }
        let derived = tool_input(spec, conn);
        check_args(&derived.bindings, &derived.schema, args).map_err(SqlError::InvalidArgs)?;
        let dry_run = args
            .get(DRY_RUN_ARG)
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let text = |k: &str| {
            args.get(k)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
        };
        let read_run = |o: NosqlOutcome| NosqlRun {
            result: Ok(o),
            audit: None,
        };
        let (op, target, body, agent) = match spec.operation.as_str() {
            LIST_TARGETS => {
                let pattern = text(NOSQL_PATTERN_ARG);
                let names = self
                    .with_client(conn, |c| async move { c.list_targets(pattern).await })
                    .await?;
                let want = pattern.map(str::to_lowercase);
                let mut documents: Vec<Value> = names
                    .into_iter()
                    .filter(|n| policy::check_target(driver, n, &conn.table_allowlist).is_ok())
                    .filter(|n| {
                        // Redis already matched the pattern as a key glob.
                        driver == Driver::Redis
                            || want
                                .as_ref()
                                .is_none_or(|p| n.to_lowercase().contains(p.as_str()))
                    })
                    .map(|n| json!({ "name": n }))
                    .collect();
                documents.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
                let truncated = documents.len() > MAX_TARGETS;
                documents.truncate(MAX_TARGETS);
                return Ok(read_run(NosqlOutcome {
                    request: json!({ "operation": LIST_TARGETS, "pattern": pattern }),
                    documents,
                    truncated,
                    ..Default::default()
                }));
            }
            DESCRIBE_TARGET => {
                let target = text(NOSQL_TARGET_ARG).unwrap_or_default().to_owned();
                policy::check_target(driver, &target, &conn.table_allowlist)
                    .map_err(SqlError::Rejected)?;
                let t = target.clone();
                let desc = self
                    .with_client(conn, |c| async move { c.describe(&t).await })
                    .await?;
                let masked: Vec<String> = conn
                    .column_masks
                    .iter()
                    .filter(|m| m.table == "*" || m.table == target)
                    .map(|m| m.column.clone())
                    .collect();
                return Ok(read_run(NosqlOutcome {
                    request: json!({ "operation": DESCRIBE_TARGET, "target": target }),
                    documents: vec![desc],
                    masked_fields: masked,
                    ..Default::default()
                }));
            }
            RUN_QUERY => {
                let op = text(NOSQL_OPERATION_ARG).unwrap_or_default().to_owned();
                let target = text(NOSQL_TARGET_ARG).unwrap_or_default().to_owned();
                let body = match args.get(NOSQL_BODY_ARG) {
                    // Agents sometimes send the body as a JSON string.
                    Some(Value::String(s)) => serde_json::from_str(s).map_err(|e| {
                        SqlError::Rejected(format!("the body isn't valid JSON: {e}"))
                    })?,
                    Some(v) => v.clone(),
                    None => Value::Null,
                };
                policy::check_masks(&conn.column_masks, &target, &body)
                    .map_err(SqlError::Rejected)?;
                (op, target, body, true)
            }
            op => {
                if driver == Driver::Mongodb {
                    // A `$name` string would be a field path in aggregations.
                    if let Some((k, _)) = args
                        .iter()
                        .find(|(_, v)| v.as_str().is_some_and(|s| s.starts_with('$')))
                    {
                        return Err(SqlError::Rejected(format!("`{k}` can't start with `$`")));
                    }
                }
                let target = forge_core::render_target(&spec.target, args);
                let body = forge_core::render_value(&spec.body, args);
                (op.to_owned(), target, body, false)
            }
        };
        let class = policy::class_of(driver, &op).map_err(SqlError::Rejected)?;
        policy::gate(class, conn.access_mode, agent).map_err(SqlError::Rejected)?;
        policy::check_target(driver, &target, &conn.table_allowlist).map_err(SqlError::Rejected)?;
        policy::check_body(driver, &body, &conn.table_allowlist).map_err(SqlError::Rejected)?;
        if driver == Driver::Redis {
            // Multi-key commands: every key must be allowed.
            for key in redis_db::extra_keys(&op, &body) {
                policy::check_target(driver, &key, &conn.table_allowlist)
                    .map_err(SqlError::Rejected)?;
            }
        }
        let request = json!({ "operation": op, "target": target, "body": body });
        let limits = Limits {
            row_limit: conn.row_limit,
            timeout_ms: conn.timeout_ms,
            max_affected: conn.max_affected_rows,
        };
        if class == OpClass::Read {
            let (o, t, b) = (op.clone(), target.clone(), body.clone());
            let docs = self
                .with_client(conn, |c| async move { c.read(&o, &t, &b, limits).await })
                .await?;
            let mut documents: Vec<Value> = docs.documents.into_iter().map(cap).collect();
            let masked_fields = policy::apply_masks(&conn.column_masks, &target, &mut documents);
            return Ok(read_run(NosqlOutcome {
                request,
                documents,
                truncated: docs.truncated,
                summary: docs.summary,
                masked_fields,
                elapsed_ms: 0,
                write: None,
            }));
        }
        // A write: audited whatever happens.
        let params: Vec<Value> = spec
            .params
            .iter()
            .map(|p| args.get(&p.name).cloned().unwrap_or(Value::Null))
            .collect();
        let (o, t, b) = (op.clone(), target.clone(), body.clone());
        let result = self
            .with_client(conn, |c| async move {
                c.write(&o, &t, &b, limits, dry_run).await
            })
            .await;
        let mut audit = AuditDraft {
            sql: request.to_string(),
            params,
            class: StatementClass::Write,
            rows_affected: None,
            outcome: AuditOutcome::Failed,
            error: None,
        };
        let result = match result {
            Ok(w) => {
                audit.rows_affected = Some(w.affected);
                audit.outcome = if w.committed {
                    AuditOutcome::Committed
                } else {
                    AuditOutcome::DryRun
                };
                let mut documents: Vec<Value> = w.documents.into_iter().map(cap).collect();
                let masked_fields =
                    policy::apply_masks(&conn.column_masks, &target, &mut documents);
                Ok(NosqlOutcome {
                    request,
                    documents,
                    truncated: false,
                    summary: None,
                    masked_fields,
                    elapsed_ms: 0,
                    write: Some(WriteInfo {
                        class: "write".into(),
                        affected_rows: w.affected,
                        committed: w.committed,
                        dry_run,
                    }),
                })
            }
            Err(e) => {
                if let SqlError::TooManyRows { affected, .. } = &e {
                    audit.rows_affected = Some(*affected);
                    audit.outcome = AuditOutcome::RolledBack;
                }
                audit.error = Some(e.to_string());
                Err(e)
            }
        };
        Ok(NosqlRun {
            result,
            audit: Some(audit),
        })
    }

    /// Resolves the DSN, gets the client and runs `f` within the time limit;
    /// errors are redacted.
    async fn with_client<T, F, Fut>(&self, conn: &DbConnection, f: F) -> Result<T, SqlError>
    where
        F: FnOnce(Client) -> Fut,
        Fut: std::future::Future<Output = Result<T, SqlError>>,
    {
        let dsn = self.resolve_dsn(conn).await?;
        let redactor = Redactor::new(&dsn);
        let budget = Duration::from_millis(u64::from(conn.timeout_ms)) + Duration::from_secs(2);
        let r = tokio::time::timeout(budget, async {
            let c = self.client(conn, &dsn).await?;
            f(c).await
        })
        .await;
        match r {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(SqlError::Timeout(_))) | Err(_) => Err(SqlError::Timeout(conn.timeout_ms)),
            Ok(Err(e)) => Err(e.redact(&redactor)),
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/lib.rs"]
mod tests;
