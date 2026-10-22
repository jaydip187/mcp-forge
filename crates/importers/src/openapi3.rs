//! OpenAPI 3.0 / 3.1 → tool IR.
//!
//! Works on the document as a JSON value (both 3.0 and 3.1 parse the same
//! way). Every operation becomes a proposed `ToolInput`; operations that can't
//! be expressed yet are kept, flagged `supported: false`, with the reason.

use std::collections::{HashMap, HashSet};

use forge_core::{
    ApiKeyLocation, AuthKind, HttpMethod, HttpSpec, ParamDef, ParamLocation, ToolInput, ToolKind,
    DEFAULT_TIMEOUT_MS,
};
use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::refs::Resolver;
use crate::{sanitize_name, unique_name, ImportError};

/// How many `$ref`s are expanded along one schema path.
pub const MAX_REF_DEPTH: usize = 8;
const MAX_DESCRIPTION_CHARS: usize = 8_000;

const METHODS: [(&str, HttpMethod); 7] = [
    ("get", HttpMethod::Get),
    ("put", HttpMethod::Put),
    ("post", HttpMethod::Post),
    ("delete", HttpMethod::Delete),
    ("options", HttpMethod::Options),
    ("head", HttpMethod::Head),
    ("patch", HttpMethod::Patch),
];

/// Headers the spec says are ignored as parameters (or that auth configs own).
const RESERVED_HEADERS: [&str; 3] = ["accept", "content-type", "authorization"];

#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Base URL to use instead of the spec's servers (for ops without their own).
    pub base_url: Option<String>,
    /// Where the spec was fetched from; resolves relative server URLs.
    pub source_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpecInfo {
    pub title: String,
    pub api_version: String,
    /// Version as written in the document (`3.1.0`, or `2.0` for Swagger).
    pub openapi: String,
    /// `openapi3` or `swagger2`.
    pub kind: String,
    /// Server URLs as written in the spec.
    pub servers: Vec<String>,
    /// First server resolved to an absolute URL, if possible.
    pub base_url: Option<String>,
    pub security_schemes: Vec<SecurityScheme>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SecurityScheme {
    pub name: String,
    /// Plain-language description, e.g. "API key in header X-API-Key".
    pub description: String,
    /// The auth config that would implement it, if mcp-forge can.
    pub suggested: Option<AuthKind>,
}

#[derive(Debug, Clone)]
pub struct BuiltOperation {
    /// Stable identity across re-imports: `"GET /pets/{petId}"`.
    pub key: String,
    pub operation_id: Option<String>,
    pub method: HttpMethod,
    pub path: String,
    pub tags: Vec<String>,
    pub summary: Option<String>,
    pub deprecated: bool,
    /// The spec says this operation needs credentials.
    pub needs_auth: bool,
    pub tool: ToolInput,
    pub supported: bool,
    pub warnings: Vec<String>,
}

/// What the preview screen shows for one operation.
#[derive(Debug, Clone, Serialize)]
pub struct OperationSummary {
    pub key: String,
    pub operation_id: Option<String>,
    pub method: HttpMethod,
    pub path: String,
    pub name: String,
    pub tags: Vec<String>,
    pub summary: Option<String>,
    pub deprecated: bool,
    pub needs_auth: bool,
    pub param_count: usize,
    pub base_url: String,
    pub supported: bool,
    pub warnings: Vec<String>,
}

impl BuiltOperation {
    pub fn summary(&self) -> OperationSummary {
        let ToolKind::Http(h) = &self.tool.kind else {
            unreachable!("importers build HTTP tools")
        };
        OperationSummary {
            key: self.key.clone(),
            operation_id: self.operation_id.clone(),
            method: self.method,
            path: self.path.clone(),
            name: self.tool.name.clone(),
            tags: self.tags.clone(),
            summary: self.summary.clone(),
            deprecated: self.deprecated,
            needs_auth: self.needs_auth,
            param_count: h.params.len() + usize::from(h.body_schema.is_some()),
            base_url: h.base_url.clone(),
            supported: self.supported,
            warnings: self.warnings.clone(),
        }
    }

    /// The `Source::OpenApi.op_id`: operationId if present, else the key.
    pub fn op_id(&self) -> String {
        self.operation_id
            .clone()
            .unwrap_or_else(|| self.key.clone())
    }
}

#[derive(Debug, Clone)]
pub struct Imported {
    pub info: SpecInfo,
    pub operations: Vec<BuiltOperation>,
}

pub fn import(doc: &Value, opts: &Options) -> Result<Imported, ImportError> {
    let openapi = doc.get("openapi").and_then(Value::as_str).unwrap_or("");
    if !openapi.starts_with("3.") {
        if doc.get("swagger").is_some() {
            return Err(ImportError::Unsupported(
                "This is a Swagger 2.0 spec. Only OpenAPI 3.x can be imported for now.".into(),
            ));
        }
        return Err(ImportError::Unsupported(
            "This doesn't look like an OpenAPI 3.x document (no `openapi: 3.x` field).".into(),
        ));
    }
    let resolver = Resolver::new(doc, MAX_REF_DEPTH);
    let source_url = opts.source_url.as_deref();

    let servers = doc.get("servers").cloned().unwrap_or(Value::Null);
    let info = SpecInfo {
        title: str_at(doc, "/info/title")
            .unwrap_or("Untitled API")
            .to_owned(),
        api_version: str_at(doc, "/info/version").unwrap_or("").to_owned(),
        openapi: openapi.to_owned(),
        kind: "openapi3".into(),
        servers: servers
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.get("url")?.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default(),
        base_url: first_server_url(&servers, source_url),
        security_schemes: security_schemes(doc),
    };
    let default_base = opts
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.trim_end_matches('/').to_owned())
        .or_else(|| info.base_url.clone())
        .unwrap_or_default();

    let global_security = doc.get("security");
    let mut taken_names = HashSet::new();
    let mut operations = Vec::new();

    let empty = Map::new();
    let paths = doc
        .get("paths")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    for (path, raw_item) in paths {
        let item = match resolver.deref(raw_item) {
            Ok(i) => i,
            Err(_) => continue,
        };
        let item_base = first_server_url(item.get("servers").unwrap_or(&Value::Null), source_url);
        for (m, method) in METHODS {
            let Some(op) = item.get(m) else { continue };
            let op_base = first_server_url(op.get("servers").unwrap_or(&Value::Null), source_url)
                .or_else(|| item_base.clone())
                .unwrap_or_else(|| default_base.clone());
            let security = op.get("security").or(global_security);
            let built = build_operation(
                &resolver,
                path,
                item,
                op,
                method,
                op_base,
                security,
                &mut taken_names,
            );
            operations.push(built);
        }
    }
    Ok(Imported { info, operations })
}

#[allow(clippy::too_many_arguments)]
fn build_operation(
    resolver: &Resolver,
    path: &str,
    item: &Value,
    op: &Value,
    method: HttpMethod,
    base_url: String,
    security: Option<&Value>,
    taken_names: &mut HashSet<String>,
) -> BuiltOperation {
    let method_str = method_name(method);
    let key = format!("{method_str} {path}");
    let operation_id = op
        .get("operationId")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let mut warnings = Vec::new();
    let mut blockers = Vec::new();

    // Tool name: operationId, else method + path; unique within the spec.
    let base_name = match &operation_id {
        Some(id) => sanitize_name(id, false),
        None => sanitize_name(&format!("{} {path}", method_str.to_lowercase()), false),
    };
    let name = unique_name(
        if base_name.is_empty() {
            "operation".into()
        } else {
            base_name
        },
        taken_names,
    );

    // Parameters: path-item level, overridden by operation level (same name + in).
    let mut merged: Vec<&Value> = Vec::new();
    for list in [item.get("parameters"), op.get("parameters")]
        .into_iter()
        .flatten()
    {
        for raw in list.as_array().into_iter().flatten() {
            let p = match resolver.deref(raw) {
                Ok(p) => p,
                Err(e) => {
                    warnings.push(format!("Skipped a parameter: {e}."));
                    continue;
                }
            };
            let id = (p.get("name"), p.get("in"));
            merged.retain(|q| (q.get("name"), q.get("in")) != id);
            merged.push(p);
        }
    }

    let mut template = path.to_owned();
    let mut params: Vec<ParamDef> = Vec::new();
    let mut taken_by_loc: HashMap<ParamLocation, HashSet<String>> = HashMap::new();

    for p in merged {
        let Some(raw_name) = p.get("name").and_then(Value::as_str) else {
            continue;
        };
        let location = match p.get("in").and_then(Value::as_str) {
            Some("path") => ParamLocation::Path,
            Some("query") => ParamLocation::Query,
            Some("header") => ParamLocation::Header,
            Some("cookie") => {
                if p.get("required") == Some(&Value::Bool(true)) {
                    blockers.push(format!(
                        "needs the cookie `{raw_name}`, and cookie parameters aren't supported"
                    ));
                } else {
                    warnings.push(format!("Skipped optional cookie parameter `{raw_name}`."));
                }
                continue;
            }
            _ => continue,
        };
        if location == ParamLocation::Header
            && RESERVED_HEADERS.contains(&raw_name.to_ascii_lowercase().as_str())
        {
            continue;
        }
        let style = p.get("style").and_then(Value::as_str);
        if style == Some("deepObject") {
            warnings.push(format!(
                "`{raw_name}` uses deepObject style; it will be sent as JSON text."
            ));
        }
        let is_array = p.pointer("/schema/type").and_then(Value::as_str) == Some("array");
        let joined = matches!(style, Some("spaceDelimited" | "pipeDelimited"))
            || (location == ParamLocation::Query && p.get("explode") == Some(&Value::Bool(false)));
        if is_array && joined {
            warnings.push(format!(
                "`{raw_name}` expects one delimited value; arrays are sent as repeated `{raw_name}=` keys."
            ));
        }
        let schema = p
            .get("schema")
            .or_else(|| first_media_schema(p.get("content")))
            .map(|s| resolver.inline_schema(s))
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({ "type": "string" }));

        let taken = taken_by_loc.entry(location).or_default();
        let safe = unique_name(nonempty(sanitize_name(raw_name, true), "param"), taken);
        let wire_name =
            (location != ParamLocation::Path && safe != raw_name).then(|| raw_name.to_owned());
        if location == ParamLocation::Path {
            let from = format!("{{{raw_name}}}");
            if !template.contains(&from) {
                warnings.push(format!(
                    "Path parameter `{raw_name}` isn't in the path; skipped."
                ));
                continue;
            }
            template = template.replace(&from, &format!("{{{safe}}}"));
        }
        params.push(ParamDef {
            name: safe,
            location,
            schema,
            required: location == ParamLocation::Path
                || p.get("required") == Some(&Value::Bool(true)),
            description: p
                .get("description")
                .and_then(Value::as_str)
                .map(str::to_owned),
            fixed_value: None,
            wire_name,
        });
    }

    // Placeholders the spec forgot to declare become required string params.
    for ph in forge_core::path_placeholders(&template).unwrap_or_default() {
        let declared = params
            .iter()
            .any(|p| p.location == ParamLocation::Path && p.name == ph);
        if !declared {
            let taken = taken_by_loc.entry(ParamLocation::Path).or_default();
            let safe = unique_name(nonempty(sanitize_name(&ph, true), "param"), taken);
            template = template.replace(&format!("{{{ph}}}"), &format!("{{{safe}}}"));
            warnings.push(format!(
                "Path placeholder `{{{ph}}}` wasn't declared; added it as a required string."
            ));
            params.push(ParamDef {
                name: safe,
                location: ParamLocation::Path,
                schema: json!({ "type": "string" }),
                required: true,
                description: None,
                fixed_value: None,
                wire_name: None,
            });
        }
    }

    // Request body.
    let mut body_schema = None;
    if let Some(raw_body) = op.get("requestBody") {
        match resolver.deref(raw_body) {
            Err(e) => blockers.push(format!("its request body can't be read ({e})")),
            Ok(rb) => {
                let required = rb.get("required") == Some(&Value::Bool(true));
                let content = rb.get("content").and_then(Value::as_object);
                match content.and_then(pick_json_media_type) {
                    Some(mt) => {
                        let schema = content
                            .and_then(|c| c.get(mt))
                            .and_then(|m| m.get("schema"))
                            .map(|s| resolver.inline_schema(s))
                            .unwrap_or_else(|| json!({ "type": "object" }));
                        match flatten_body(&schema) {
                            Some(props) => {
                                let taken = taken_by_loc.entry(ParamLocation::Body).or_default();
                                for (raw_name, prop_schema, prop_required) in props {
                                    let safe = unique_name(
                                        nonempty(sanitize_name(&raw_name, true), "field"),
                                        taken,
                                    );
                                    let description = prop_schema
                                        .get("description")
                                        .and_then(Value::as_str)
                                        .map(str::to_owned);
                                    params.push(ParamDef {
                                        wire_name: (safe != raw_name).then(|| raw_name.clone()),
                                        name: safe,
                                        location: ParamLocation::Body,
                                        schema: prop_schema,
                                        required: prop_required && required,
                                        description,
                                        fixed_value: None,
                                    });
                                }
                            }
                            None => {
                                body_schema = Some(if schema.is_object() {
                                    schema
                                } else {
                                    json!({})
                                })
                            }
                        }
                    }
                    None => {
                        let types: Vec<&str> = content
                            .map(|c| c.keys().map(String::as_str).collect())
                            .unwrap_or_default();
                        let types = if types.is_empty() {
                            "unspecified".to_owned()
                        } else {
                            types.join(", ")
                        };
                        if required {
                            blockers.push(format!(
                                "its request body is {types}; only JSON bodies are supported"
                            ));
                        } else {
                            warnings.push(format!("Skipped the optional {types} request body."));
                        }
                    }
                }
            }
        }
    }

    let summary = op
        .get("summary")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    let deprecated = op.get("deprecated") == Some(&Value::Bool(true));
    let description = compose_description(
        summary.as_deref(),
        op.get("description").and_then(Value::as_str),
        deprecated,
    );

    let tags: Vec<String> = op
        .get("tags")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .filter(|t| !t.trim().is_empty())
                .take(forge_core::MAX_TAGS)
                .map(|t| t.chars().take(64).collect())
                .collect()
        })
        .unwrap_or_default();
    let tool = ToolInput {
        tags: tags.clone(),
        name,
        description,
        kind: ToolKind::Http(HttpSpec {
            method,
            base_url: base_url.clone(),
            path_template: template,
            params,
            body_schema,
            auth_ref: None,
            timeout_ms: DEFAULT_TIMEOUT_MS,
        }),
        enabled: true,
        response_shape: None,
    };

    if base_url.is_empty() {
        blockers.push("there's no base URL; enter one above".into());
    }
    let mut supported = blockers.is_empty();
    if supported {
        if let Err(errs) = tool.validate() {
            supported = false;
            for e in errs.errors {
                warnings.push(format!("{}: {}", e.field, e.message));
            }
        }
    }
    for b in blockers {
        warnings.insert(0, format!("Can't import: {b}."));
    }

    BuiltOperation {
        key,
        operation_id,
        method,
        path: path.to_owned(),
        tags,
        summary,
        deprecated,
        needs_auth: needs_auth(security),
        tool,
        supported,
        warnings,
    }
}

/// Splits a plain JSON object schema into (name, schema, required) fields.
/// Composed schemas (allOf/oneOf/anyOf) and non-objects stay whole.
fn flatten_body(schema: &Value) -> Option<Vec<(String, Value, bool)>> {
    let obj = schema.as_object()?;
    let is_object = match obj.get("type") {
        Some(Value::String(t)) => t == "object",
        None => obj.contains_key("properties"),
        _ => false,
    };
    if !is_object
        || ["allOf", "oneOf", "anyOf", "not"]
            .iter()
            .any(|k| obj.contains_key(*k))
    {
        return None;
    }
    let props = obj.get("properties")?.as_object()?;
    if props.is_empty() {
        return None;
    }
    let required: HashSet<&str> = obj
        .get("required")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let fields: Vec<_> = props
        .iter()
        .filter(|(_, s)| s.get("readOnly") != Some(&Value::Bool(true)))
        .map(|(k, s)| {
            let s = if s.is_object() { s.clone() } else { json!({}) };
            (k.clone(), s, required.contains(k.as_str()))
        })
        .collect();
    (!fields.is_empty()).then_some(fields)
}

fn pick_json_media_type(content: &Map<String, Value>) -> Option<&str> {
    let keys: Vec<&str> = content.keys().map(String::as_str).collect();
    keys.iter()
        .find(|k| k.split(';').next().map(str::trim) == Some("application/json"))
        .or_else(|| {
            keys.iter()
                .find(|k| k.contains("+json") || k.ends_with("/json"))
        })
        .or_else(|| keys.iter().find(|k| **k == "*/*"))
        .copied()
}

fn first_media_schema(content: Option<&Value>) -> Option<&Value> {
    content?.as_object()?.values().find_map(|m| m.get("schema"))
}

fn compose_description(
    summary: Option<&str>,
    description: Option<&str>,
    deprecated: bool,
) -> String {
    let mut parts = Vec::new();
    if deprecated {
        parts.push("Deprecated.".to_owned());
    }
    if let Some(s) = summary {
        parts.push(s.to_owned());
    }
    if let Some(d) = description.map(str::trim).filter(|d| !d.is_empty()) {
        if Some(d) != summary {
            parts.push(d.to_owned());
        }
    }
    let mut text = parts.join("\n\n");
    if text.chars().count() > MAX_DESCRIPTION_CHARS {
        text = text
            .chars()
            .take(MAX_DESCRIPTION_CHARS - 1)
            .collect::<String>()
            + "…";
    }
    text
}

fn needs_auth(security: Option<&Value>) -> bool {
    // `security: []` or `[{}]` means no auth (or optional auth).
    security.and_then(Value::as_array).is_some_and(|reqs| {
        !reqs.is_empty()
            && reqs
                .iter()
                .all(|r| r.as_object().is_some_and(|o| !o.is_empty()))
    })
}

fn first_server_url(servers: &Value, source_url: Option<&str>) -> Option<String> {
    let server = servers.as_array()?.first()?;
    let mut url = server.get("url")?.as_str()?.to_owned();
    if let Some(vars) = server.get("variables").and_then(Value::as_object) {
        for (k, v) in vars {
            if let Some(d) = v.get("default").and_then(Value::as_str) {
                url = url.replace(&format!("{{{k}}}"), d);
            }
        }
    }
    let absolute = match url::Url::parse(&url) {
        Ok(u) => u,
        Err(url::ParseError::RelativeUrlWithoutBase) => {
            url::Url::parse(source_url?).ok()?.join(&url).ok()?
        }
        Err(_) => return None,
    };
    if !matches!(absolute.scheme(), "http" | "https") || absolute.host_str().is_none() {
        return None;
    }
    let mut s = absolute.to_string();
    if absolute.query().is_some() || absolute.fragment().is_some() {
        let mut clean = absolute.clone();
        clean.set_query(None);
        clean.set_fragment(None);
        s = clean.to_string();
    }
    Some(s.trim_end_matches('/').to_owned())
}

fn security_schemes(doc: &Value) -> Vec<SecurityScheme> {
    let Some(schemes) = doc
        .pointer("/components/securitySchemes")
        .and_then(Value::as_object)
    else {
        return vec![];
    };
    schemes
        .iter()
        .map(|(name, s)| {
            let t = s.get("type").and_then(Value::as_str).unwrap_or("");
            let (description, suggested) = match t {
                "apiKey" => {
                    let key = s
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned();
                    match s.get("in").and_then(Value::as_str) {
                        Some("header") => (
                            format!("API key in header {key}"),
                            Some(AuthKind::ApiKey {
                                location: ApiKeyLocation::Header,
                                name: key,
                            }),
                        ),
                        Some("query") => (
                            format!("API key in query parameter {key}"),
                            Some(AuthKind::ApiKey {
                                location: ApiKeyLocation::Query,
                                name: key,
                            }),
                        ),
                        _ => (format!("API key in cookie {key} (not supported)"), None),
                    }
                }
                "http" => match s
                    .get("scheme")
                    .and_then(Value::as_str)
                    .map(str::to_ascii_lowercase)
                    .as_deref()
                {
                    Some("bearer") => ("Bearer token".into(), Some(AuthKind::Bearer)),
                    Some("basic") => (
                        "Basic auth".into(),
                        Some(AuthKind::Basic {
                            username: String::new(),
                        }),
                    ),
                    other => (
                        format!("HTTP {} auth (not supported)", other.unwrap_or("?")),
                        None,
                    ),
                },
                "oauth2" | "openIdConnect" => (
                    "OAuth 2 (paste an access token as a bearer token; no sign-in flow)".into(),
                    Some(AuthKind::Bearer),
                ),
                other => (format!("{other} (not supported)"), None),
            };
            SecurityScheme {
                name: name.clone(),
                description,
                suggested,
            }
        })
        .collect()
}

fn method_name(m: HttpMethod) -> &'static str {
    match m {
        HttpMethod::Get => "GET",
        HttpMethod::Post => "POST",
        HttpMethod::Put => "PUT",
        HttpMethod::Patch => "PATCH",
        HttpMethod::Delete => "DELETE",
        HttpMethod::Head => "HEAD",
        HttpMethod::Options => "OPTIONS",
    }
}

fn str_at<'a>(v: &'a Value, pointer: &str) -> Option<&'a str> {
    v.pointer(pointer).and_then(Value::as_str)
}

fn nonempty(s: String, fallback: &str) -> String {
    if s.is_empty() {
        fallback.to_owned()
    } else {
        s
    }
}
