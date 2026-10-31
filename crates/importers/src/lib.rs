//! Importers that turn API descriptions into the canonical tool IR.
//!
//! - [`openapi3`]: OpenAPI 3.0 and 3.1 (JSON or YAML).
//!
//! - [`swagger2`]: Swagger 2.0, converted to OpenAPI 3.0 and imported by the same code.
//! - [`postman`], [`har`], [`curl`]: converted to OpenAPI 3.0 without secrets
//!   (see [`convert`]), then imported the same way.

pub mod convert;
pub mod curl;
pub mod graphql;
pub mod har;
pub mod openapi3;
pub mod postman;
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
    let mut imported = openapi3::import(doc, opts)?;
    if let Some(origin) = doc
        .pointer("/info")
        .and_then(|i| i.get(convert::ORIGIN_KEY))
        .and_then(Value::as_str)
    {
        imported.info.kind = origin.to_owned();
    }
    Ok(imported)
}

/// Imports any supported text: OpenAPI 3.x or Swagger 2.0 (JSON/YAML), a
/// Postman collection, a HAR file, or curl commands. For the converted
/// formats the second value is the OpenAPI document (no secrets) to keep
/// instead of the original.
pub fn import_text(
    text: &str,
    opts: &openapi3::Options,
) -> Result<(openapi3::Imported, Option<String>), ImportError> {
    if graphql::is_sdl(text) {
        return Ok((graphql::import_sdl(text, opts)?, None));
    }
    let (doc, converted) = if curl::is_curl(text) {
        (curl::to_openapi3(text)?, true)
    } else {
        let doc = parse_document(text).map_err(|e| match e {
            ImportError::Parse(m) => ImportError::Parse(format!(
                "{m}. Expected an OpenAPI or Swagger spec, a Postman collection, a HAR file, or curl commands."
            )),
            other => other,
        })?;
        if graphql::is_introspection(&doc) {
            return Ok((graphql::import_introspection(&doc, opts)?, None));
        }
        if postman::is_postman(&doc) {
            (postman::to_openapi3(&doc)?, true)
        } else if har::is_har(&doc) {
            (har::to_openapi3(&doc)?, true)
        } else {
            (doc, false)
        }
    };
    let imported = import_any(&doc, opts)?;
    let text = converted.then(|| serde_json::to_string_pretty(&doc).expect("JSON serializes"));
    Ok((imported, text))
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
#[path = "../tests/unit/lib.rs"]
mod tests;
