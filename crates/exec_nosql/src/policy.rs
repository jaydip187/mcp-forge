//! What a NoSQL request may do: operation vs access mode, targets vs the
//! allowlist, per-engine body rules, and masked fields.

use forge_core::{op_class, AccessMode, ColumnMask, DbConnectionInput, Driver, OpClass};
use serde_json::Value;

use forge_exec_sql::MASKED;

/// Simple glob: `*` any run of characters, `?` one character.
pub(crate) fn glob(pattern: &str, name: &str) -> bool {
    fn go(p: &[char], n: &[char]) -> bool {
        match (p.first(), n.first()) {
            (None, None) => true,
            (Some('*'), _) => go(&p[1..], n) || (!n.is_empty() && go(p, &n[1..])),
            (Some('?'), Some(_)) => go(&p[1..], &n[1..]),
            (Some(a), Some(b)) if a == b => go(&p[1..], &n[1..]),
            _ => false,
        }
    }
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    go(&p, &n)
}

/// Whether the access mode lets `class` run. Agent-written queries
/// (`run_query`) may only write under full access.
pub fn gate(class: OpClass, mode: AccessMode, agent: bool) -> Result<(), String> {
    match (class, mode, agent) {
        (OpClass::Read, _, _) => Ok(()),
        (OpClass::Write, AccessMode::FullAccess, _) => Ok(()),
        (OpClass::Write, AccessMode::CuratedWrite, false) => Ok(()),
        (OpClass::Write, AccessMode::CuratedWrite, true) => Err(
            "this connection only allows writes through saved tools; run_query is read-only".into(),
        ),
        (OpClass::Write, AccessMode::ReadOnly, _) => Err("this connection is read-only".into()),
    }
}

/// The class of `op`, or why it isn't offered.
pub fn class_of(driver: Driver, op: &str) -> Result<OpClass, String> {
    op_class(driver, op).ok_or_else(|| {
        let names: Vec<&str> = forge_core::nosql_operations(driver)
            .iter()
            .map(|(n, _)| *n)
            .collect();
        format!("`{op}` isn't available; use one of: {}", names.join(", "))
    })
}

/// System collections, indexes and keys that are never available.
pub fn is_system_target(driver: Driver, target: &str) -> bool {
    match driver {
        Driver::Mongodb => target.starts_with("system."),
        Driver::Elasticsearch => target.starts_with('.') || target.starts_with('_'),
        _ => false,
    }
}

/// Checks a (rendered) target against the allowlist. Empty allows every
/// non-system target. Elasticsearch targets can't widen to other indexes.
pub fn check_target(driver: Driver, target: &str, allowlist: &[String]) -> Result<(), String> {
    if target.is_empty() {
        return Err("the target must not be empty".into());
    }
    if is_system_target(driver, target) {
        return Err(format!("`{target}` is a system name and isn't available"));
    }
    if driver == Driver::Elasticsearch
        && (target.contains(',') || target.contains('*') || target == "_all")
        && !allowlist.iter().any(|a| a == target)
    {
        return Err("use one index (no commas or wildcards) unless that pattern is allowed".into());
    }
    if allowlist.is_empty() || allowlist.iter().any(|a| glob(a, target)) {
        return Ok(());
    }
    Err(format!(
        "`{target}` isn't available through this connection; allowed: {}",
        allowlist.join(", ")
    ))
}

/// MongoDB stages and operators that run code, write elsewhere or read
/// server state.
const MONGO_DENIED: &[&str] = &[
    "$where",
    "$function",
    "$accumulator",
    "$out",
    "$merge",
    "$currentOp",
    "$listSessions",
    "$listLocalSessions",
    "$planCacheStats",
    "$listSearchIndexes",
    "$changeStream",
];

/// Stages that read another collection: the name must be allowed too.
const MONGO_OTHER_COLLECTION: &[(&str, &str)] = &[
    ("$lookup", "from"),
    ("$graphLookup", "from"),
    ("$unionWith", "coll"),
];

/// Elasticsearch keys that run scripts.
const ELASTIC_DENIED: &[&str] = &[
    "script",
    "script_fields",
    "runtime_mappings",
    "scripted_metric",
];

fn walk_keys(
    v: &Value,
    f: &mut dyn FnMut(&str, &Value) -> Result<(), String>,
) -> Result<(), String> {
    match v {
        Value::Object(o) => {
            for (k, x) in o {
                f(k, x)?;
                walk_keys(x, f)?;
            }
            Ok(())
        }
        Value::Array(a) => a.iter().try_for_each(|x| walk_keys(x, f)),
        _ => Ok(()),
    }
}

/// Per-engine rules for a request body (curated or agent-written).
pub fn check_body(driver: Driver, body: &Value, allowlist: &[String]) -> Result<(), String> {
    match driver {
        Driver::Mongodb => walk_keys(body, &mut |k, v| {
            if MONGO_DENIED.contains(&k) {
                return Err(format!("`{k}` is not allowed"));
            }
            if let Some((_, field)) = MONGO_OTHER_COLLECTION.iter().find(|(s, _)| *s == k) {
                let coll = match v {
                    Value::String(s) => Some(s.as_str()),
                    Value::Object(o) => o.get(*field).and_then(Value::as_str),
                    _ => None,
                };
                match coll {
                    Some(c) => check_target(driver, c, allowlist)?,
                    // `$lookup` with a pipeline and no `from` reads nothing new.
                    None if k == "$lookup" => {}
                    None => return Err(format!("`{k}` needs the collection name")),
                }
            }
            Ok(())
        }),
        Driver::Elasticsearch => walk_keys(body, &mut |k, _| {
            if ELASTIC_DENIED.contains(&k) {
                Err(format!("`{k}` is not allowed (scripts can't run)"))
            } else {
                Ok(())
            }
        }),
        Driver::Dynamodb => match body {
            Value::Object(o) if o.contains_key("TableName") => {
                Err("leave out TableName; the target names the table".into())
            }
            Value::Object(_) | Value::Null => Ok(()),
            _ => Err("the body must be a JSON object".into()),
        },
        Driver::Redis => match body {
            Value::Array(a) if a.iter().all(|x| !x.is_object() && !x.is_array()) => Ok(()),
            Value::Null => Ok(()),
            _ => Err("the body must be an array of plain arguments".into()),
        },
        _ => Err("not a NoSQL connection".into()),
    }
}

/// Save-time checks for a curated NoSQL tool against its connection.
pub fn check_tool(
    spec: &forge_core::NosqlSpec,
    conn: &DbConnectionInput,
) -> Result<(), forge_core::ValidationErrors> {
    let mut errs = forge_core::ValidationErrors::default();
    if conn.driver.is_sql() {
        errs.push(
            "kind.connection_id",
            "pick a NoSQL connection; this one runs SQL tools",
        );
        return errs.into_result();
    }
    if spec.is_exploration() {
        return Ok(());
    }
    match class_of(conn.driver, &spec.operation) {
        Err(e) => errs.push("kind.operation", e),
        Ok(class) => {
            if let Err(e) = gate(class, conn.access_mode, false) {
                errs.push("kind.operation", e);
            }
        }
    }
    // A templated target is checked when it runs.
    if !spec.target.contains("{{") {
        if let Err(e) = check_target(conn.driver, &spec.target, &conn.table_allowlist) {
            errs.push("kind.target", e);
        }
    }
    if let Err(e) = check_body(conn.driver, &spec.body, &conn.table_allowlist) {
        errs.push("kind.body", e);
    }
    errs.into_result()
}

/// Masks that apply to a target (`target.field` or `*.field`).
fn masks_for<'a>(masks: &'a [ColumnMask], target: &str) -> Vec<&'a str> {
    masks
        .iter()
        .filter(|m| m.table == "*" || m.table == target)
        .map(|m| m.column.as_str())
        .collect()
}

/// Replaces masked fields (at any depth) in place; returns the names hit.
pub fn apply_masks(masks: &[ColumnMask], target: &str, docs: &mut [Value]) -> Vec<String> {
    let fields = masks_for(masks, target);
    let mut hit: Vec<String> = Vec::new();
    fn go(v: &mut Value, fields: &[&str], hit: &mut Vec<String>) {
        match v {
            Value::Object(o) => {
                for (k, x) in o.iter_mut() {
                    if fields.iter().any(|f| f.eq_ignore_ascii_case(k)) {
                        if !x.is_null() {
                            *x = Value::String(MASKED.into());
                        }
                        if !hit.contains(k) {
                            hit.push(k.clone());
                        }
                    } else {
                        go(x, fields, hit);
                    }
                }
            }
            Value::Array(a) => a.iter_mut().for_each(|x| go(x, fields, hit)),
            _ => {}
        }
    }
    if !fields.is_empty() {
        docs.iter_mut().for_each(|d| go(d, &fields, &mut hit));
    }
    hit
}

/// Agent-written bodies can't mention a masked field at all (as a key, or
/// as a `$field` reference), so they can't filter or compute on it.
pub fn check_masks(masks: &[ColumnMask], target: &str, body: &Value) -> Result<(), String> {
    let fields = masks_for(masks, target);
    if fields.is_empty() {
        return Ok(());
    }
    fn mentions(v: &Value, fields: &[&str]) -> Option<String> {
        let hit = |s: &str| {
            let s = s.trim_start_matches('$');
            fields
                .iter()
                .find(|f| {
                    s.eq_ignore_ascii_case(f) || s.split('.').any(|p| p.eq_ignore_ascii_case(f))
                })
                .map(|f| (*f).to_owned())
        };
        match v {
            Value::Object(o) => o
                .iter()
                .find_map(|(k, x)| hit(k).or_else(|| mentions(x, fields))),
            Value::Array(a) => a.iter().find_map(|x| mentions(x, fields)),
            Value::String(s) if s.starts_with('$') => hit(s),
            _ => None,
        }
    }
    match mentions(body, &fields) {
        Some(f) => Err(format!("`{f}` is masked and can't be used in a query")),
        None => Ok(()),
    }
}

#[cfg(test)]
#[path = "../tests/unit/policy.rs"]
mod tests;
