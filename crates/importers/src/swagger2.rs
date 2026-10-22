//! Swagger 2.0 → OpenAPI 3.0 conversion, so 2.0 specs go through the same
//! importer (and come out the same) as 3.x ones.
//!
//! Covers what tool import needs: servers from `schemes`/`host`/`basePath`,
//! `definitions`, `securityDefinitions`, body/formData parameters (→
//! `requestBody`), `consumes`, and `collectionFormat`. Response objects are
//! not needed and are left out.

use serde_json::{json, Map, Value};

use crate::refs::Resolver;
use crate::ImportError;

/// Converts a Swagger 2.0 document. Fails if `swagger` isn't `"2.0"`.
pub fn to_openapi3(doc: &Value) -> Result<Value, ImportError> {
    if doc.get("swagger").and_then(Value::as_str) != Some("2.0") {
        return Err(ImportError::Unsupported(
            "This isn't a Swagger 2.0 document.".into(),
        ));
    }
    let resolver = Resolver::new(doc, 0);
    let mut out = Map::new();
    out.insert("openapi".into(), json!("3.0.3"));
    out.insert(
        "info".into(),
        doc.get("info").cloned().unwrap_or_else(|| json!({})),
    );
    if let Some(server) = server_url(doc) {
        out.insert("servers".into(), json!([{ "url": server }]));
    }
    if let Some(sec) = doc.get("security") {
        out.insert("security".into(), sec.clone());
    }

    let mut components = Map::new();
    if let Some(defs) = doc.get("definitions") {
        components.insert("schemas".into(), rewrite_refs(defs));
    }
    if let Some(defs) = doc.get("securityDefinitions").and_then(Value::as_object) {
        let schemes: Map<String, Value> = defs
            .iter()
            .map(|(k, v)| (k.clone(), convert_security(v)))
            .collect();
        components.insert("securitySchemes".into(), Value::Object(schemes));
    }
    out.insert("components".into(), Value::Object(components));

    let global_consumes = doc.get("consumes");
    let mut paths = Map::new();
    for (path, raw_item) in doc
        .get("paths")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        let Ok(item) = resolver.deref(raw_item) else {
            continue;
        };
        let Some(item) = item.as_object() else {
            continue;
        };
        let shared_params: Vec<&Value> = item
            .get("parameters")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|p| resolver.deref(p).ok()).collect())
            .unwrap_or_default();
        let mut new_item = Map::new();
        for (method, op) in item {
            if matches!(method.as_str(), "parameters" | "$ref") || method.starts_with("x-") {
                continue;
            }
            let Some(op) = op.as_object() else { continue };
            let consumes = op.get("consumes").or(global_consumes);
            let own: Vec<&Value> = op
                .get("parameters")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|p| resolver.deref(p).ok()).collect())
                .unwrap_or_default();
            // Operation params override path-level ones with the same name + in.
            let mut params: Vec<&Value> = shared_params
                .iter()
                .copied()
                .filter(|s| {
                    !own.iter()
                        .any(|o| o.get("name") == s.get("name") && o.get("in") == s.get("in"))
                })
                .collect();
            params.extend(own);
            new_item.insert(method.clone(), convert_operation(op, &params, consumes));
        }
        paths.insert(path.clone(), Value::Object(new_item));
    }
    out.insert("paths".into(), Value::Object(paths));
    Ok(Value::Object(out))
}

fn convert_operation(
    op: &Map<String, Value>,
    params: &[&Value],
    consumes: Option<&Value>,
) -> Value {
    let mut out = Map::new();
    for key in [
        "operationId",
        "summary",
        "description",
        "tags",
        "deprecated",
        "security",
    ] {
        if let Some(v) = op.get(key) {
            out.insert(key.into(), v.clone());
        }
    }

    let mut new_params = Vec::new();
    let mut body: Option<&Value> = None;
    let mut form_props = Map::new();
    let mut form_required = Vec::new();
    let mut has_file = false;
    for p in params {
        match p.get("in").and_then(Value::as_str) {
            Some("body") => body = Some(p),
            Some("formData") => {
                let name = p
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("field")
                    .to_owned();
                has_file |= p.get("type").and_then(Value::as_str) == Some("file");
                if p.get("required") == Some(&Value::Bool(true)) {
                    form_required.push(Value::String(name.clone()));
                }
                form_props.insert(name, param_schema(p));
            }
            Some(loc @ ("path" | "query" | "header")) => new_params.push(convert_param(p, loc)),
            _ => {}
        }
    }
    if !new_params.is_empty() {
        out.insert("parameters".into(), Value::Array(new_params));
    }

    let media_types: Vec<&str> = consumes
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if let Some(b) = body {
        // Prefer a JSON media type; 2.0 defaults to JSON when `consumes` is absent.
        let media = media_types
            .iter()
            .find(|m| m.contains("json"))
            .or(media_types.first())
            .copied()
            .unwrap_or("application/json");
        let schema = b
            .get("schema")
            .map(rewrite_refs)
            .unwrap_or_else(|| json!({}));
        let mut rb = json!({ "content": { media: { "schema": schema } } });
        if b.get("required") == Some(&Value::Bool(true)) {
            rb["required"] = json!(true);
        }
        if let Some(d) = b.get("description") {
            rb["description"] = d.clone();
        }
        out.insert("requestBody".into(), rb);
    } else if !form_props.is_empty() {
        let media = if has_file || media_types.contains(&"multipart/form-data") {
            "multipart/form-data"
        } else {
            "application/x-www-form-urlencoded"
        };
        let mut schema = json!({ "type": "object", "properties": form_props });
        if !form_required.is_empty() {
            schema["required"] = Value::Array(form_required.clone());
        }
        let mut rb = json!({ "content": { media: { "schema": schema } } });
        if !form_required.is_empty() {
            rb["required"] = json!(true);
        }
        out.insert("requestBody".into(), rb);
    }
    Value::Object(out)
}

fn convert_param(p: &Value, location: &str) -> Value {
    let mut out = json!({
        "name": p.get("name").cloned().unwrap_or(json!("param")),
        "in": location,
        "schema": param_schema(p),
    });
    if location == "path" || p.get("required") == Some(&Value::Bool(true)) {
        out["required"] = json!(true);
    }
    if let Some(d) = p.get("description") {
        out["description"] = d.clone();
    }
    // collectionFormat → style/explode (2.0 defaults to csv, i.e. not exploded).
    if p.get("type").and_then(Value::as_str) == Some("array") {
        let (style, explode) = match p
            .get("collectionFormat")
            .and_then(Value::as_str)
            .unwrap_or("csv")
        {
            "multi" => ("form", true),
            "ssv" => ("spaceDelimited", false),
            "pipes" => ("pipeDelimited", false),
            _ if location == "query" => ("form", false),
            _ => ("simple", false),
        };
        out["style"] = json!(style);
        out["explode"] = json!(explode);
    }
    out
}

/// A non-body 2.0 parameter carries its schema keywords inline.
fn param_schema(p: &Value) -> Value {
    const KEYS: [&str; 16] = [
        "type",
        "format",
        "items",
        "enum",
        "default",
        "minimum",
        "maximum",
        "exclusiveMinimum",
        "exclusiveMaximum",
        "minLength",
        "maxLength",
        "pattern",
        "minItems",
        "maxItems",
        "uniqueItems",
        "multipleOf",
    ];
    let mut s = Map::new();
    for k in KEYS {
        if let Some(v) = p.get(k) {
            s.insert(k.into(), rewrite_refs(v));
        }
    }
    if s.get("type").and_then(Value::as_str) == Some("file") {
        s.insert("type".into(), json!("string"));
        s.insert("format".into(), json!("binary"));
    }
    if p.get("x-nullable") == Some(&Value::Bool(true)) {
        s.insert("nullable".into(), json!(true));
    }
    if s.is_empty() {
        s.insert("type".into(), json!("string"));
    }
    Value::Object(s)
}

fn convert_security(s: &Value) -> Value {
    match s.get("type").and_then(Value::as_str) {
        Some("basic") => json!({ "type": "http", "scheme": "basic" }),
        Some("apiKey") => json!({ "type": "apiKey", "in": s.get("in"), "name": s.get("name") }),
        Some("oauth2") => json!({ "type": "oauth2", "flows": {} }),
        _ => s.clone(),
    }
}

/// `https://host/basePath`, preferring https when several schemes are listed.
/// Without `host`, the base path alone (resolved later against the fetch URL).
fn server_url(doc: &Value) -> Option<String> {
    let base_path = doc.get("basePath").and_then(Value::as_str).unwrap_or("");
    let base_path = base_path.trim_end_matches('/');
    match doc.get("host").and_then(Value::as_str) {
        Some(host) if !host.is_empty() => {
            let schemes: Vec<&str> = doc
                .get("schemes")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let scheme = if schemes.contains(&"https") || schemes.is_empty() {
                "https"
            } else {
                schemes[0]
            };
            Some(format!("{scheme}://{host}{base_path}"))
        }
        _ if !base_path.is_empty() => Some(base_path.to_owned()),
        _ => None,
    }
}

/// Rewrites 2.0 refs (`#/definitions/X`) to 3.0 (`#/components/schemas/X`)
/// and `x-nullable` to `nullable`, everywhere inside a schema.
fn rewrite_refs(v: &Value) -> Value {
    match v {
        Value::Object(obj) => {
            let mut out = Map::with_capacity(obj.len());
            for (k, child) in obj {
                if k == "$ref" {
                    if let Some(r) = child.as_str() {
                        let r = r.replacen("#/definitions/", "#/components/schemas/", 1);
                        out.insert(k.clone(), Value::String(r));
                        continue;
                    }
                }
                if k == "x-nullable" {
                    out.insert("nullable".into(), child.clone());
                    continue;
                }
                let child = if matches!(k.as_str(), "example" | "default" | "enum") {
                    child.clone()
                } else {
                    rewrite_refs(child)
                };
                out.insert(k.clone(), child);
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(rewrite_refs).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
#[path = "tests/swagger2.rs"]
mod tests;
