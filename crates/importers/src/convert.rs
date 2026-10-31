//! Postman collections, HAR captures and curl commands, converted to an
//! OpenAPI 3.0 document so the OpenAPI importer (preview, filters, naming,
//! base URL, auth, re-import) handles them unchanged.
//!
//! The converted document never carries secrets: cookies, header and query
//! values with secret-looking names, and example values from request bodies
//! are left out. Authorization headers and secret-looking keys become
//! security schemes, so the user picks an auth config instead. The server
//! stores the converted document, not the original (a HAR file can hold
//! session cookies).

use std::collections::BTreeMap;

use forge_core::is_secret_name;
use serde_json::{json, Map, Value};

/// Marks a converted document so a later import (or re-import) knows its origin.
pub const ORIGIN_KEY: &str = "x-mcp-forge-converted-from";

/// Headers that never become parameters: transport, browser and auth noise.
const DROPPED_HEADERS: &[&str] = &[
    "authorization",
    "cookie",
    "content-type",
    "content-length",
    "accept",
    "accept-encoding",
    "accept-language",
    "user-agent",
    "host",
    "connection",
    "origin",
    "referer",
    "cache-control",
    "pragma",
    "upgrade-insecure-requests",
    "x-requested-with",
    "dnt",
    "te",
    "priority",
    "if-none-match",
    "if-modified-since",
];

pub(crate) fn header_dropped(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    DROPPED_HEADERS.contains(&n.as_str()) || n.starts_with("sec-") || n.starts_with(':')
}

/// A header, query or path parameter of a converted operation.
#[derive(Debug, Clone, Default)]
pub(crate) struct Param {
    pub name: String,
    /// Shown to the agent as the default (never for secret-looking names).
    pub default: Option<String>,
    pub required: bool,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) enum Body {
    #[default]
    None,
    /// JSON, described by a schema inferred from an example (types only).
    Json(Value),
    /// Anything else: kept as a media type so the importer can say why the
    /// operation isn't supported.
    Other(String),
}

/// One request, format-neutral.
#[derive(Debug, Clone, Default)]
pub(crate) struct Op {
    pub method: String,
    /// `scheme://host[:port]`, if known.
    pub base: Option<String>,
    /// Path with `{name}` placeholders.
    pub path: String,
    pub path_params: Vec<Param>,
    pub query: Vec<Param>,
    pub headers: Vec<Param>,
    pub body: Body,
    pub operation_id: Option<String>,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub tags: Vec<String>,
    /// Names of security schemes that apply; `None` = the document default.
    pub security: Option<Vec<String>>,
}

/// A converted collection: operations plus the security schemes they use.
#[derive(Debug, Default)]
pub(crate) struct Collection {
    pub title: String,
    pub description: Option<String>,
    pub ops: Vec<Op>,
    pub schemes: BTreeMap<String, Value>,
    /// Applies to every operation without its own `security`.
    pub default_security: Option<Vec<String>>,
}

impl Collection {
    /// Records the scheme an `Authorization` header or a secret-looking
    /// header/query key implies, returning its name.
    pub fn scheme_for_header(&mut self, name: &str, value: &str) -> Option<String> {
        let lower = name.to_ascii_lowercase();
        // Cookies are browser sessions, not a way to call an API: drop them.
        if lower == "cookie" || lower == "set-cookie" {
            return None;
        }
        if lower == "authorization" {
            let v = value.trim_start().to_ascii_lowercase();
            if v.starts_with("bearer ") {
                return Some(
                    self.add_scheme("bearerAuth", json!({ "type": "http", "scheme": "bearer" })),
                );
            }
            if v.starts_with("basic ") {
                return Some(
                    self.add_scheme("basicAuth", json!({ "type": "http", "scheme": "basic" })),
                );
            }
            return Some(self.add_scheme(
                "authorizationHeader",
                json!({ "type": "apiKey", "in": "header", "name": "Authorization" }),
            ));
        }
        is_secret_name(name).then(|| self.api_key_scheme(name, "header"))
    }

    pub fn api_key_scheme(&mut self, name: &str, location: &str) -> String {
        let id: String = format!("{location}_{name}")
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        self.add_scheme(
            &id,
            json!({ "type": "apiKey", "in": location, "name": name }),
        )
    }

    pub fn add_scheme(&mut self, id: &str, scheme: Value) -> String {
        self.schemes.entry(id.to_owned()).or_insert(scheme);
        id.to_owned()
    }

    /// The OpenAPI 3.0 document.
    pub fn into_openapi(self, origin: &str) -> Value {
        // The most common base URL is the document's server; others go on
        // their operations.
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for op in &self.ops {
            if let Some(b) = &op.base {
                *counts.entry(b).or_default() += 1;
            }
        }
        let main = counts
            .iter()
            .max_by_key(|(b, n)| (**n, std::cmp::Reverse(**b)))
            .map(|(b, _)| (*b).to_owned());

        let mut paths: Map<String, Value> = Map::new();
        for op in &self.ops {
            let method = op.method.to_ascii_lowercase();
            let item = paths
                .entry(op.path.clone())
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .expect("path items are objects");
            if item.contains_key(&method) {
                // Same method and path seen before (HAR repeats, duplicate
                // Postman requests): the first one wins.
                continue;
            }
            item.insert(method, operation(op, main.as_deref()));
        }

        let mut info = json!({ "title": self.title, "version": "" });
        if let Some(d) = &self.description {
            info["description"] = json!(d);
        }
        info[ORIGIN_KEY] = json!(origin);
        let mut doc = json!({ "openapi": "3.0.3", "info": info, "paths": paths });
        if let Some(m) = main {
            doc["servers"] = json!([{ "url": m }]);
        }
        if !self.schemes.is_empty() {
            doc["components"] = json!({ "securitySchemes": self.schemes });
        }
        if let Some(sec) = &self.default_security {
            doc["security"] = security_list(sec);
        }
        doc
    }
}

fn security_list(names: &[String]) -> Value {
    Value::Array(names.iter().map(|n| json!({ n.clone(): [] })).collect())
}

fn param_json(p: &Param, location: &str) -> Value {
    let mut schema = json!({ "type": "string" });
    if let Some(d) = &p.default {
        schema["default"] = json!(d);
    }
    let mut v = json!({
        "name": p.name,
        "in": location,
        "required": p.required || location == "path",
        "schema": schema,
    });
    if let Some(d) = &p.description {
        v["description"] = json!(d);
    }
    v
}

fn operation(op: &Op, main: Option<&str>) -> Value {
    let mut o = Map::new();
    if let Some(id) = &op.operation_id {
        o.insert("operationId".into(), json!(id));
    }
    if let Some(s) = &op.summary {
        o.insert("summary".into(), json!(s));
    }
    if let Some(d) = &op.description {
        o.insert("description".into(), json!(d));
    }
    if !op.tags.is_empty() {
        o.insert("tags".into(), json!(op.tags));
    }
    let mut params = vec![];
    params.extend(op.path_params.iter().map(|p| param_json(p, "path")));
    params.extend(op.query.iter().map(|p| param_json(p, "query")));
    params.extend(op.headers.iter().map(|p| param_json(p, "header")));
    if !params.is_empty() {
        o.insert("parameters".into(), Value::Array(params));
    }
    match &op.body {
        Body::None => {}
        Body::Json(schema) => {
            o.insert(
                "requestBody".into(),
                json!({ "required": true, "content": { "application/json": { "schema": schema } } }),
            );
        }
        Body::Other(mime) => {
            o.insert(
                "requestBody".into(),
                json!({ "required": true, "content": { mime.clone(): { "schema": {} } } }),
            );
        }
    }
    if let (Some(b), Some(m)) = (&op.base, main) {
        if b != m {
            o.insert("servers".into(), json!([{ "url": b }]));
        }
    } else if let (Some(b), None) = (&op.base, main) {
        o.insert("servers".into(), json!([{ "url": b }]));
    }
    if let Some(sec) = &op.security {
        o.insert("security".into(), security_list(sec));
    }
    o.insert(
        "responses".into(),
        json!({ "200": { "description": "Response" } }),
    );
    Value::Object(o)
}

/// A schema describing `example` by type only (values can be secrets).
pub(crate) fn infer_schema(example: &Value) -> Value {
    match example {
        Value::Object(m) => {
            let props: Map<String, Value> = m
                .iter()
                .map(|(k, v)| (k.clone(), infer_schema(v)))
                .collect();
            json!({ "type": "object", "properties": props })
        }
        Value::Array(items) => match items.first() {
            Some(first) => json!({ "type": "array", "items": infer_schema(first) }),
            None => json!({ "type": "array", "items": {} }),
        },
        Value::String(_) => json!({ "type": "string" }),
        Value::Bool(_) => json!({ "type": "boolean" }),
        Value::Number(n) if n.is_i64() || n.is_u64() => json!({ "type": "integer" }),
        Value::Number(_) => json!({ "type": "number" }),
        Value::Null => json!({}),
    }
}

/// Body description from raw text and an optional content type.
pub(crate) fn body_from(text: &str, content_type: Option<&str>) -> Body {
    let text = text.trim();
    if text.is_empty() {
        return Body::None;
    }
    let ct = content_type.map(|c| c.to_ascii_lowercase());
    let jsonish = ct.as_deref().is_none_or(|c| c.contains("json"));
    if jsonish {
        if let Ok(v) = serde_json::from_str::<Value>(text) {
            return Body::Json(infer_schema(&v));
        }
    }
    Body::Other(match ct {
        Some(c) => c.split(';').next().unwrap_or("").trim().to_owned(),
        None if text.contains('=') => "application/x-www-form-urlencoded".into(),
        None => "text/plain".into(),
    })
}

/// A query or header parameter from a name/value pair seen in a request.
/// Secret-looking names become a security scheme instead (returned as Err).
pub(crate) fn sample_param(name: &str, value: Option<&str>, keep_value: bool) -> Param {
    Param {
        name: name.to_owned(),
        default: value
            .filter(|_| keep_value && !is_secret_name(name))
            .filter(|v| !v.is_empty() && !v.contains("{{"))
            .map(str::to_owned),
        required: false,
        description: None,
    }
}

/// Splits a URL into base (`scheme://host[:port]`) and path; `None` base
/// when it can't be told (relative URL, unresolved variables in the host).
pub(crate) fn split_url(raw: &str) -> Option<(Option<String>, url::Url)> {
    let u = url::Url::parse(raw).ok()?;
    let base = u.host_str().map(|h| {
        let port = u.port().map(|p| format!(":{p}")).unwrap_or_default();
        format!("{}://{h}{port}", u.scheme())
    });
    Some((base, u))
}

/// Unique placeholder name for a path segment (`id`, `id2`, …).
pub(crate) fn unique_param(base: &str, taken: &[Param]) -> String {
    let clean: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let clean = if clean.is_empty() {
        "id".to_owned()
    } else {
        clean
    };
    let mut name = clean.clone();
    let mut n = 2;
    while taken.iter().any(|p| p.name == name) {
        name = format!("{clean}{n}");
        n += 1;
    }
    name
}

#[cfg(test)]
#[path = "../tests/unit/convert.rs"]
mod tests;
