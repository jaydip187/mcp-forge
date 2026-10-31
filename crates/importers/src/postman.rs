//! Postman Collection v2.0 / v2.1 → OpenAPI 3.0 (see [`crate::convert`]).
//!
//! Folders become tags; collection variables (`{{name}}`) are filled in
//! where they have values, and unresolved ones in a path become path
//! parameters (like `:id`). Auth blocks become security schemes.

use std::collections::HashMap;

use forge_core::is_secret_name;
use serde_json::{json, Value};

use crate::convert::{
    body_from, header_dropped, sample_param, unique_param, Body, Collection, Op, Param,
};
use crate::ImportError;

/// Whether `doc` looks like a Postman collection.
pub fn is_postman(doc: &Value) -> bool {
    let schema = doc
        .pointer("/info/schema")
        .and_then(Value::as_str)
        .unwrap_or("");
    schema.contains("getpostman.com")
        || (doc.pointer("/info/_postman_id").is_some() && doc.get("item").is_some())
}

pub fn to_openapi3(doc: &Value) -> Result<Value, ImportError> {
    let vars: HashMap<String, String> = doc
        .get("variable")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| {
            let k = v.get("key")?.as_str()?;
            let val = v.get("value").map(value_text)?;
            // Variables with secret-looking names stay unresolved.
            (!val.is_empty() && !is_secret_name(k)).then(|| (k.to_owned(), val))
        })
        .collect();
    let mut c = Collection {
        title: doc
            .pointer("/info/name")
            .and_then(Value::as_str)
            .unwrap_or("Postman collection")
            .to_owned(),
        description: doc
            .pointer("/info/description")
            .map(description)
            .filter(|d| !d.is_empty()),
        ..Collection::default()
    };
    if let Some(auth) = doc.get("auth") {
        c.default_security = auth_security(&mut c, auth);
    }
    let items = doc.get("item").and_then(Value::as_array).ok_or_else(|| {
        ImportError::Unsupported("The Postman collection has no requests.".into())
    })?;
    walk(&mut c, items, &[], &vars);
    if c.ops.is_empty() {
        return Err(ImportError::Unsupported(
            "The Postman collection has no requests.".into(),
        ));
    }
    Ok(c.into_openapi("postman"))
}

fn value_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn description(v: &Value) -> String {
    match v {
        Value::String(s) => s.trim().to_owned(),
        Value::Object(o) => o
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_owned(),
        _ => String::new(),
    }
}

/// Replaces `{{name}}` with known variables; unknown ones stay as written.
fn resolve(text: &str, vars: &HashMap<String, String>) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find("}}") {
            Some(end) => {
                let name = after[..end].trim();
                match vars.get(name) {
                    Some(v) => out.push_str(v),
                    None => {
                        out.push_str("{{");
                        out.push_str(&after[..end]);
                        out.push_str("}}");
                    }
                }
                rest = &after[end + 2..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// The security a Postman `auth` block implies (`noauth` → none).
fn auth_security(c: &mut Collection, auth: &Value) -> Option<Vec<String>> {
    let kind = auth.get("type").and_then(Value::as_str)?;
    let scheme = match kind {
        "noauth" => return Some(vec![]),
        "bearer" | "jwt" => {
            c.add_scheme("bearerAuth", json!({ "type": "http", "scheme": "bearer" }))
        }
        "basic" => c.add_scheme("basicAuth", json!({ "type": "http", "scheme": "basic" })),
        "apikey" => {
            let field = |k: &str| {
                auth.get("apikey")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .find(|e| e.get("key").and_then(Value::as_str) == Some(k))
                    .and_then(|e| e.get("value"))
                    .map(value_text)
            };
            let name = field("key")
                .filter(|k| !k.is_empty())
                .unwrap_or_else(|| "X-API-Key".into());
            let location = if field("in").as_deref() == Some("query") {
                "query"
            } else {
                "header"
            };
            c.api_key_scheme(&name, location)
        }
        "oauth2" => c.add_scheme("oauth2", json!({ "type": "oauth2", "flows": {} })),
        // Digest, AWS, NTLM, Hawk…: not supported; agents would need a bearer token.
        _ => return None,
    };
    Some(vec![scheme])
}

fn walk(c: &mut Collection, items: &[Value], folders: &[String], vars: &HashMap<String, String>) {
    for item in items {
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_owned();
        if let Some(children) = item.get("item").and_then(Value::as_array) {
            let mut path = folders.to_vec();
            if !name.is_empty() {
                path.push(name);
            }
            walk(c, children, &path, vars);
            continue;
        }
        if let Some(req) = item.get("request") {
            if let Some(op) = request(c, req, &name, folders, vars) {
                c.ops.push(op);
            }
        }
    }
}

fn request(
    c: &mut Collection,
    req: &Value,
    name: &str,
    folders: &[String],
    vars: &HashMap<String, String>,
) -> Option<Op> {
    // v2.x allows a request to be just a URL string.
    let (method, url, headers, body, auth, desc) = match req {
        Value::String(u) => ("GET".to_owned(), json!(u), None, None, None, None),
        Value::Object(o) => (
            o.get("method")
                .and_then(Value::as_str)
                .unwrap_or("GET")
                .to_ascii_uppercase(),
            o.get("url").cloned().unwrap_or(Value::Null),
            o.get("header"),
            o.get("body"),
            o.get("auth"),
            o.get("description"),
        ),
        _ => return None,
    };
    let raw = match &url {
        Value::String(s) => s.clone(),
        Value::Object(u) => u
            .get("raw")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| {
                // Rebuild from parts when `raw` is missing.
                let host = match u.get("host") {
                    Some(Value::Array(h)) => h.iter().map(value_text).collect::<Vec<_>>().join("."),
                    Some(v) => value_text(v),
                    None => String::new(),
                };
                let path = match u.get("path") {
                    Some(Value::Array(p)) => p.iter().map(value_text).collect::<Vec<_>>().join("/"),
                    Some(v) => value_text(v),
                    None => String::new(),
                };
                let proto = u
                    .get("protocol")
                    .map(value_text)
                    .unwrap_or_else(|| "https".into());
                format!("{proto}://{host}/{path}")
            }),
        _ => return None,
    };
    let resolved = resolve(&raw, vars);
    // Unresolved variables in the host leave the base URL to the user.
    let (base, path, query_pairs) = parse_raw(&resolved);

    let mut op = Op {
        method,
        base,
        operation_id: (!name.is_empty()).then(|| name.to_owned()),
        summary: (!name.is_empty()).then(|| name.to_owned()),
        description: desc.map(description).filter(|d| !d.is_empty()),
        tags: folders.to_vec(),
        ..Op::default()
    };

    // Path: `:name` and unresolved `{{name}}` become parameters.
    let mut segs = vec![];
    for seg in path.split('/').filter(|s| !s.is_empty()) {
        let var = seg
            .strip_prefix(':')
            .or_else(|| seg.strip_prefix("{{").and_then(|s| s.strip_suffix("}}")));
        match var {
            Some(v) => {
                let pname = unique_param(v.trim(), &op.path_params);
                let doc = url
                    .get("variable")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .find(|x| x.get("key").and_then(Value::as_str) == Some(v))
                    .and_then(|x| x.get("description"))
                    .map(description)
                    .filter(|d| !d.is_empty());
                segs.push(format!("{{{pname}}}"));
                op.path_params.push(Param {
                    name: pname,
                    required: true,
                    description: doc,
                    ..Param::default()
                });
            }
            None => segs.push(seg.to_owned()),
        }
    }
    op.path = format!("/{}", segs.join("/"));

    // Query: from the URL object when present (it knows disabled params).
    let query: Vec<(String, Option<String>, Option<String>)> =
        match url.get("query").and_then(Value::as_array) {
            Some(q) => q
                .iter()
                .filter(|p| p.get("disabled").and_then(Value::as_bool) != Some(true))
                .filter_map(|p| {
                    let k = p.get("key")?.as_str()?.to_owned();
                    let v = p.get("value").map(value_text).map(|v| resolve(&v, vars));
                    Some((k, v, p.get("description").map(description)))
                })
                .collect(),
            None => query_pairs
                .into_iter()
                .map(|(k, v)| (k, Some(v), None))
                .collect(),
        };
    for (k, v, d) in query {
        if is_secret_name(&k) {
            let s = c.api_key_scheme(&k, "query");
            op.security.get_or_insert_with(Vec::new).push(s);
            continue;
        }
        let mut p = sample_param(&k, v.as_deref(), true);
        p.description = d.filter(|d| !d.is_empty());
        op.query.push(p);
    }

    let mut content_type = None;
    for h in headers.and_then(Value::as_array).into_iter().flatten() {
        if h.get("disabled").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let (Some(k), v) = (
            h.get("key").and_then(Value::as_str),
            h.get("value").map(value_text),
        ) else {
            continue;
        };
        let v = v.map(|v| resolve(&v, vars)).unwrap_or_default();
        if k.eq_ignore_ascii_case("content-type") {
            content_type = Some(v.clone());
        }
        if let Some(s) = c.scheme_for_header(k, &v) {
            op.security.get_or_insert_with(Vec::new).push(s);
            continue;
        }
        if header_dropped(k) {
            continue;
        }
        op.headers.push(sample_param(k, Some(&v), true));
    }

    if let Some(b) = body {
        op.body = match b.get("mode").and_then(Value::as_str) {
            Some("raw") => {
                let lang = b.pointer("/options/raw/language").and_then(Value::as_str);
                let ct = content_type
                    .clone()
                    .or_else(|| (lang == Some("json")).then(|| "application/json".into()));
                body_from(
                    b.get("raw").and_then(Value::as_str).unwrap_or(""),
                    ct.as_deref(),
                )
            }
            Some("urlencoded") => Body::Other("application/x-www-form-urlencoded".into()),
            Some("formdata") => Body::Other("multipart/form-data".into()),
            Some("graphql") => Body::Other("application/graphql".into()),
            Some("file") => Body::Other("application/octet-stream".into()),
            _ => Body::None,
        };
    }

    if let Some(a) = auth {
        if let Some(sec) = auth_security(c, a) {
            op.security = Some(sec);
        }
    }
    Some(op)
}

/// `(base, path, query)` of a raw URL that may still contain `{{vars}}`.
/// A host that is (or contains) a variable leaves the base URL unknown.
fn parse_raw(raw: &str) -> (Option<String>, String, Vec<(String, String)>) {
    let (scheme, rest) = raw.split_once("://").unwrap_or(("https", raw));
    let rest = rest.split('#').next().unwrap_or("");
    let (before_query, query_part) = rest.split_once('?').unwrap_or((rest, ""));
    let (host, path) = match before_query.find('/') {
        Some(i) => (&before_query[..i], &before_query[i..]),
        None => (before_query, "/"),
    };
    let base = (!host.is_empty() && !host.contains("{{")).then(|| format!("{scheme}://{host}"));
    let query = url::form_urlencoded::parse(query_part.as_bytes())
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    (base, path.to_owned(), query)
}
