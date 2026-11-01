//! NoSQL tools: one operation of a MongoDB, Redis, DynamoDB or
//! Elasticsearch connection, with `{{param}}` templates in its target and
//! body, plus the opt-in exploration tools (`list_targets`,
//! `describe_target`, `run_query`).
//!
//! Templates only ever produce JSON values: `"{{id}}"` as a whole string is
//! replaced by the (typed, scalar) argument, `"user:{{id}}"` by text. Object
//! keys are never templated, so an argument can't add a query operator.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::db::{is_valid_sql_param_name, Driver, SqlParam, SQL_PARAM_TYPES};
use crate::schema::{ArgBinding, ArgTarget, DerivedInput};
use crate::validate::ValidationErrors;

/// Whether an operation reads or changes data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpClass {
    Read,
    Write,
}

use OpClass::{Read, Write};

const MONGO_OPS: &[(&str, OpClass)] = &[
    ("find", Read),
    ("find_one", Read),
    ("aggregate", Read),
    ("count", Read),
    ("distinct", Read),
    ("insert_one", Write),
    ("update_one", Write),
    ("update_many", Write),
    ("delete_one", Write),
    ("delete_many", Write),
];

const REDIS_OPS: &[(&str, OpClass)] = &[
    ("GET", Read),
    ("MGET", Read),
    ("STRLEN", Read),
    ("EXISTS", Read),
    ("TYPE", Read),
    ("TTL", Read),
    ("HGET", Read),
    ("HMGET", Read),
    ("HGETALL", Read),
    ("HKEYS", Read),
    ("HLEN", Read),
    ("LRANGE", Read),
    ("LLEN", Read),
    ("LINDEX", Read),
    ("SMEMBERS", Read),
    ("SISMEMBER", Read),
    ("SCARD", Read),
    ("ZRANGE", Read),
    ("ZREVRANGE", Read),
    ("ZRANGEBYSCORE", Read),
    ("ZSCORE", Read),
    ("ZCARD", Read),
    ("XRANGE", Read),
    ("XREVRANGE", Read),
    ("XLEN", Read),
    ("SET", Write),
    ("DEL", Write),
    ("EXPIRE", Write),
    ("INCR", Write),
    ("INCRBY", Write),
    ("HSET", Write),
    ("HDEL", Write),
    ("HINCRBY", Write),
    ("LPUSH", Write),
    ("RPUSH", Write),
    ("SADD", Write),
    ("SREM", Write),
    ("ZADD", Write),
    ("ZREM", Write),
];

const DYNAMO_OPS: &[(&str, OpClass)] = &[
    ("GetItem", Read),
    ("Query", Read),
    ("Scan", Read),
    ("PutItem", Write),
    ("UpdateItem", Write),
    ("DeleteItem", Write),
];

const ELASTIC_OPS: &[(&str, OpClass)] = &[
    ("search", Read),
    ("count", Read),
    ("get", Read),
    ("index", Write),
    ("update", Write),
    ("delete", Write),
];

/// The operations a NoSQL driver offers; empty for SQL drivers.
pub fn nosql_operations(driver: Driver) -> &'static [(&'static str, OpClass)] {
    match driver {
        Driver::Mongodb => MONGO_OPS,
        Driver::Redis => REDIS_OPS,
        Driver::Dynamodb => DYNAMO_OPS,
        Driver::Elasticsearch => ELASTIC_OPS,
        _ => &[],
    }
}

/// Class of `op` on `driver`; Redis commands match case-insensitively.
pub fn op_class(driver: Driver, op: &str) -> Option<OpClass> {
    nosql_operations(driver)
        .iter()
        .find(|(name, _)| {
            if driver == Driver::Redis {
                name.eq_ignore_ascii_case(op)
            } else {
                *name == op
            }
        })
        .map(|(_, c)| *c)
}

/// Exploration tools (one per connection, opt-in).
pub const LIST_TARGETS: &str = "list_targets";
pub const DESCRIBE_TARGET: &str = "describe_target";
pub const RUN_QUERY: &str = "run_query";
pub const EXPLORATION_OPS: [&str; 3] = [LIST_TARGETS, DESCRIBE_TARGET, RUN_QUERY];

/// Argument names of the exploration tools.
pub const NOSQL_PATTERN_ARG: &str = "pattern";
pub const NOSQL_TARGET_ARG: &str = "target";
pub const NOSQL_OPERATION_ARG: &str = "operation";
pub const NOSQL_BODY_ARG: &str = "body";

const MAX_BODY_LEN: usize = 100_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NosqlSpec {
    pub connection_id: Uuid,
    /// An operation of the connection's driver (`find`, `HGETALL`, `Query`,
    /// `search`, …) or an exploration tool (`list_targets`, …).
    pub operation: String,
    /// Collection, key, table or index. Templates allowed.
    #[serde(default)]
    pub target: String,
    /// Filter / pipeline / request / query (an object; Redis: an array of
    /// extra arguments). Templates allowed in values.
    #[serde(default)]
    pub body: Value,
    #[serde(default)]
    pub params: Vec<SqlParam>,
}

/// `{{name}}` placeholders in a string, in order.
pub(crate) fn placeholders_in(s: &str, out: &mut Vec<String>) {
    let mut rest = s;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else { break };
        let name = after[..end].trim();
        if !name.is_empty() && !out.iter().any(|n| n == name) {
            out.push(name.to_owned());
        }
        rest = &after[end + 2..];
    }
}

fn placeholders_value(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) => placeholders_in(s, out),
        Value::Array(a) => a.iter().for_each(|x| placeholders_value(x, out)),
        Value::Object(o) => o.values().for_each(|x| placeholders_value(x, out)),
        _ => {}
    }
}

/// An argument as text inside a longer string.
fn as_text(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

/// The whole-string placeholder name, if `s` is exactly `{{name}}`.
fn whole(s: &str) -> Option<&str> {
    let inner = s.trim().strip_prefix("{{")?.strip_suffix("}}")?;
    (!inner.contains("{{") && !inner.contains("}}")).then(|| inner.trim())
}

pub(crate) fn render_str(s: &str, args: &Map<String, Value>) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else { break };
        out.push_str(&rest[..start]);
        out.push_str(&as_text(args.get(after[..end].trim())));
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

/// Fills templates in `v` from `args` (already checked against the params).
pub fn render_value(v: &Value, args: &Map<String, Value>) -> Value {
    match v {
        Value::String(s) => match whole(s) {
            Some(name) => args.get(name).cloned().unwrap_or(Value::Null),
            None if s.contains("{{") => Value::String(render_str(s, args)),
            None => v.clone(),
        },
        Value::Array(a) => Value::Array(a.iter().map(|x| render_value(x, args)).collect()),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, x)| (k.clone(), render_value(x, args)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Fills templates in a target string.
pub fn render_target(s: &str, args: &Map<String, Value>) -> String {
    render_str(s, args)
}

impl NosqlSpec {
    pub fn is_exploration(&self) -> bool {
        EXPLORATION_OPS.contains(&self.operation.as_str())
    }

    /// Placeholder names used in the target and body.
    pub fn placeholders(&self) -> Vec<String> {
        let mut out = Vec::new();
        placeholders_in(&self.target, &mut out);
        placeholders_value(&self.body, &mut out);
        out
    }

    /// Whether this tool changes data on `driver`. Exploration tools are
    /// decided per connection (see the executor).
    pub fn writes(&self, driver: Driver) -> bool {
        op_class(driver, &self.operation) == Some(OpClass::Write)
    }

    pub fn derive_input(&self) -> DerivedInput {
        let bind = |name: &str, required: bool| ArgBinding {
            arg: name.to_owned(),
            target: ArgTarget::SqlParam {
                name: name.to_owned(),
            },
            required,
        };
        let (properties, required, bindings): (Value, Vec<&str>, Vec<ArgBinding>) = match self
            .operation
            .as_str()
        {
            LIST_TARGETS => (
                json!({ NOSQL_PATTERN_ARG: { "type": "string",
                        "description": "Only names containing this text (case-insensitive; Redis: a key pattern like `user:*`)." } }),
                vec![],
                vec![bind(NOSQL_PATTERN_ARG, false)],
            ),
            DESCRIBE_TARGET => (
                json!({ NOSQL_TARGET_ARG: { "type": "string",
                        "description": "Collection, table or index name (Redis: a key)." } }),
                vec![NOSQL_TARGET_ARG],
                vec![bind(NOSQL_TARGET_ARG, true)],
            ),
            RUN_QUERY => (
                json!({
                    NOSQL_OPERATION_ARG: { "type": "string", "description": "The operation to run." },
                    NOSQL_TARGET_ARG: { "type": "string", "description": "Collection, table or index (Redis: the key)." },
                    NOSQL_BODY_ARG: { "description": "The operation's body as JSON (Redis: an array of extra arguments)." }
                }),
                vec![NOSQL_OPERATION_ARG, NOSQL_TARGET_ARG],
                vec![
                    bind(NOSQL_OPERATION_ARG, true),
                    bind(NOSQL_TARGET_ARG, true),
                    bind(NOSQL_BODY_ARG, false),
                ],
            ),
            _ => {
                let mut props = Map::new();
                let mut req = Vec::new();
                let mut binds = Vec::new();
                for p in &self.params {
                    let mut schema = p.schema.clone();
                    if let (Some(desc), Some(obj)) = (&p.description, schema.as_object_mut()) {
                        obj.entry("description")
                            .or_insert_with(|| Value::String(desc.clone()));
                    }
                    props.insert(p.name.clone(), schema);
                    if p.required {
                        req.push(p.name.as_str());
                    }
                    binds.push(bind(&p.name, p.required));
                }
                (Value::Object(props), req, binds)
            }
        };
        let Value::Object(schema) = json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        }) else {
            unreachable!("json! object literal")
        };
        DerivedInput { schema, bindings }
    }

    /// Structural checks. The operation's fit with the driver, targets and
    /// body rules need the connection and run in the NoSQL executor.
    pub(crate) fn validate_into(&self, errs: &mut ValidationErrors) {
        if self.operation.trim().is_empty() {
            errs.push("kind.operation", "pick an operation");
        }
        if self.is_exploration() {
            return;
        }
        if self.target.trim().is_empty() {
            errs.push("kind.target", "must not be empty");
        }
        if self.body.to_string().len() > MAX_BODY_LEN {
            errs.push("kind.body", format!("must be at most {MAX_BODY_LEN} bytes"));
        }
        if !matches!(self.body, Value::Object(_) | Value::Array(_) | Value::Null) {
            errs.push("kind.body", "must be a JSON object (Redis: an array)");
        }
        let used = self.placeholders();
        let mut seen = HashSet::new();
        for (i, p) in self.params.iter().enumerate() {
            let at = |f: &str| format!("kind.params[{i}].{f}");
            if !is_valid_sql_param_name(&p.name) {
                errs.push(
                    at("name"),
                    "must start with a letter or _ and contain only a-z, A-Z, 0-9 or _ (max 64)",
                );
            }
            if !seen.insert(p.name.to_lowercase()) {
                errs.push(at("name"), format!("duplicate param `{}`", p.name));
            }
            if !used.contains(&p.name) {
                errs.push(
                    at("name"),
                    format!(
                        "`{}` isn't used; write {{{{{}}}}} in the target or body",
                        p.name, p.name
                    ),
                );
            }
            match p.schema.get("type").and_then(Value::as_str) {
                _ if !p.schema.is_object() => {
                    errs.push(at("schema"), "must be a JSON Schema object")
                }
                Some(t) if SQL_PARAM_TYPES.contains(&t) => {}
                None => {}
                Some(t) => errs.push(
                    at("schema"),
                    format!("type `{t}` is not supported; use string, integer, number or boolean"),
                ),
            }
        }
        for name in used {
            if !self.params.iter().any(|p| p.name == name) {
                errs.push(
                    "kind.params",
                    format!("{{{{{name}}}}} has no matching param"),
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/nosql.rs"]
mod tests;
