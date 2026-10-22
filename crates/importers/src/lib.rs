//! Importers that turn API descriptions into the canonical tool IR.
//!
//! - [`openapi3`]: OpenAPI 3.0 and 3.1 (JSON or YAML).
//!
//! - [`swagger2`]: Swagger 2.0, converted to OpenAPI 3.0 and imported by the same code.

pub mod openapi3;
mod refs;
pub mod swagger2;

use std::collections::HashSet;

use forge_core::MAX_NAME_LEN;
use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("Couldn't read the file as JSON or YAML: {0}")]
    Parse(String),
    #[error("{0}")]
    Unsupported(String),
}

/// Imports an OpenAPI 3.x or Swagger 2.0 document.
pub fn import_any(
    doc: &Value,
    opts: &openapi3::Options,
) -> Result<openapi3::Imported, ImportError> {
    if doc.get("swagger").is_some() {
        let converted = swagger2::to_openapi3(doc)?;
        let mut imported = openapi3::import(&converted, opts)?;
        imported.info.openapi = "2.0".into();
        imported.info.kind = "swagger2".into();
        return Ok(imported);
    }
    openapi3::import(doc, opts)
}

/// Parses JSON or YAML into a JSON value. YAML keys that aren't strings
/// (e.g. response codes like `200:`) become strings.
pub fn parse_document(text: &str) -> Result<Value, ImportError> {
    let text = text.trim_start_matches('\u{feff}');
    if text.trim_start().starts_with('{') {
        return serde_json::from_str(text).map_err(|e| ImportError::Parse(e.to_string()));
    }
    let yaml: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(text).map_err(|e| ImportError::Parse(e.to_string()))?;
    let v = yaml_to_json(yaml);
    if v.is_object() {
        Ok(v)
    } else {
        Err(ImportError::Parse("the document isn't an object".into()))
    }
}

fn yaml_to_json(v: serde_yaml_ng::Value) -> Value {
    use serde_yaml_ng::Value as Y;
    match v {
        Y::Null => Value::Null,
        Y::Bool(b) => Value::Bool(b),
        Y::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::from(i)
            } else if let Some(u) = n.as_u64() {
                Value::from(u)
            } else {
                n.as_f64().map_or(Value::Null, Value::from)
            }
        }
        Y::String(s) => Value::String(s),
        Y::Sequence(items) => Value::Array(items.into_iter().map(yaml_to_json).collect()),
        Y::Mapping(m) => Value::Object(
            m.into_iter()
                .map(|(k, v)| {
                    let key = match k {
                        Y::String(s) => s,
                        Y::Number(n) => n.to_string(),
                        Y::Bool(b) => b.to_string(),
                        other => serde_yaml_ng::to_string(&other)
                            .unwrap_or_default()
                            .trim()
                            .to_owned(),
                    };
                    (key, yaml_to_json(v))
                })
                .collect(),
        ),
        Y::Tagged(t) => yaml_to_json(t.value),
    }
}

/// Maps anything to an MCP-safe name: `[A-Za-z0-9_-]` (plus `.` when
/// `allow_dot`), other runs become one `_`, at most 64 chars.
pub fn sanitize_name(raw: &str, allow_dot: bool) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        let ok = c.is_ascii_alphanumeric() || c == '-' || c == '_' || (allow_dot && c == '.');
        if ok {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('_') {
            out.push('_');
        }
    }
    let mut out = out.trim_end_matches('_').to_owned();
    out.truncate(MAX_NAME_LEN);
    out
}

/// Returns `base`, or `base_2`, `base_3`… (kept within 64 chars), and records it.
pub fn unique_name(base: String, taken: &mut HashSet<String>) -> String {
    if taken.insert(base.clone()) {
        return base;
    }
    for n in 2.. {
        let suffix = format!("_{n}");
        let mut stem = base.clone();
        stem.truncate(MAX_NAME_LEN - suffix.len());
        let candidate = format!("{stem}{suffix}");
        if taken.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!("ran out of suffixes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes() {
        assert_eq!(
            sanitize_name("repos/get-content", false),
            "repos_get-content"
        );
        assert_eq!(sanitize_name("filter[name]", true), "filter_name");
        assert_eq!(sanitize_name("$top", true), "top");
        assert_eq!(sanitize_name("a.b", true), "a.b");
        assert_eq!(sanitize_name("a.b", false), "a_b");
        assert_eq!(sanitize_name("get /pets/{petId}", false), "get_pets_petId");
        assert_eq!(sanitize_name("日本", true), "");
        assert_eq!(sanitize_name(&"x".repeat(100), false).len(), 64);
    }

    #[test]
    fn dedupes() {
        let mut taken = HashSet::new();
        assert_eq!(unique_name("a".into(), &mut taken), "a");
        assert_eq!(unique_name("a".into(), &mut taken), "a_2");
        assert_eq!(unique_name("a".into(), &mut taken), "a_3");
        let long = "y".repeat(64);
        assert_eq!(unique_name(long.clone(), &mut taken), long);
        let second = unique_name(long, &mut taken);
        assert_eq!(second.len(), 64);
        assert!(second.ends_with("_2"));
    }

    #[test]
    fn parses_json_and_yaml() {
        let j = parse_document("{\"openapi\":\"3.1.0\"}").unwrap();
        assert_eq!(j["openapi"], "3.1.0");
        let y = parse_document("openapi: 3.0.0\npaths:\n  /a:\n    get:\n      responses:\n        200:\n          description: ok\n").unwrap();
        assert_eq!(
            y["paths"]["/a"]["get"]["responses"]["200"]["description"],
            "ok"
        );
        assert!(parse_document("just a string").is_err());
        assert!(parse_document("{ nope").is_err());
    }
}
