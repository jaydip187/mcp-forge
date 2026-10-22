use std::collections::{BTreeSet, HashSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::validate::{is_valid_name, ValidationErrors, MAX_NAME_LEN};

pub const DEFAULT_TIMEOUT_MS: u32 = 30_000;
pub const MAX_TIMEOUT_MS: u32 = 300_000;
pub const MAX_DESCRIPTION_LEN: usize = 8_192;
pub const MAX_TAGS: usize = 20;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDef {
    pub id: Uuid,
    pub project_id: Uuid,
    /// MCP-safe, unique per project.
    pub name: String,
    pub description: String,
    pub kind: ToolKind,
    pub enabled: bool,
    /// Grouping labels (from the spec's tags on import); used by project filters.
    #[serde(default)]
    pub tags: Vec<String>,
    pub source: Source,
    pub overrides: Overrides,
    /// jaq expression applied to responses (Stage 7).
    pub response_shape: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Create/replace payload for a tool. `source` and `overrides` are managed by
/// the system, not supplied by clients.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub kind: ToolKind,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub response_shape: Option<String>,
}

fn default_true() -> bool {
    true
}

/// What a tool does. SQL tools are added in Stage 8.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolKind {
    Http(HttpSpec),
}

impl ToolKind {
    /// The auth config this tool depends on, if any.
    pub fn auth_ref(&self) -> Option<Uuid> {
        match self {
            ToolKind::Http(h) => h.auth_ref,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Source {
    #[default]
    Manual,
    OpenApi {
        spec_id: Uuid,
        op_id: String,
    },
    Swagger {
        spec_id: Uuid,
        op_id: String,
    },
}

impl Source {
    /// The spec an imported tool came from.
    pub fn spec_id(&self) -> Option<Uuid> {
        match self {
            Source::Manual => None,
            Source::OpenApi { spec_id, .. } | Source::Swagger { spec_id, .. } => Some(*spec_id),
        }
    }

    pub fn op_id(&self) -> Option<&str> {
        match self {
            Source::Manual => None,
            Source::OpenApi { op_id, .. } | Source::Swagger { op_id, .. } => Some(op_id),
        }
    }
}

/// Names of fields the user edited by hand; preserved across re-import (Stage 6).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Overrides {
    #[serde(default)]
    pub fields: BTreeSet<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
    Head,
    Options,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HttpSpec {
    pub method: HttpMethod,
    /// Scheme + host (+ optional base path), e.g. `https://api.example.com/v1`.
    pub base_url: String,
    /// Appended to `base_url`; `{name}` placeholders bind to path params.
    pub path_template: String,
    #[serde(default)]
    pub params: Vec<ParamDef>,
    /// JSON Schema for the whole request body. Mutually exclusive with body params.
    #[serde(default)]
    pub body_schema: Option<Value>,
    #[serde(default)]
    pub auth_ref: Option<Uuid>,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u32,
}

fn default_timeout() -> u32 {
    DEFAULT_TIMEOUT_MS
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamLocation {
    Path,
    Query,
    Header,
    Body,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParamDef {
    pub name: String,
    pub location: ParamLocation,
    /// JSON Schema for the value.
    #[serde(default = "default_param_schema")]
    pub schema: Value,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub description: Option<String>,
    /// When set, the value is always sent and the param is hidden from the agent.
    #[serde(default)]
    pub fixed_value: Option<Value>,
    /// Name sent upstream when it differs from the MCP-safe `name` (e.g.
    /// `filter[name]`). Not allowed on path params, whose placeholder is `{name}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wire_name: Option<String>,
}

impl ParamDef {
    /// The name used in the outgoing request.
    pub fn wire(&self) -> &str {
        self.wire_name.as_deref().unwrap_or(&self.name)
    }
}

fn default_param_schema() -> Value {
    serde_json::json!({ "type": "string" })
}

/// `[a-zA-Z0-9_.-]{1,64}`: what MCP clients accept as input-schema property keys.
pub fn is_valid_param_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_NAME_LEN
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.')
}

/// Extracts `{name}` placeholders from a path template, in order.
pub fn path_placeholders(template: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find(['{', '}']) {
        if rest.as_bytes()[open] == b'}' {
            return Err("unmatched `}`".into());
        }
        let after = &rest[open + 1..];
        let close = after.find('}').ok_or("unclosed `{`")?;
        let name = &after[..close];
        if name.contains('{') {
            return Err("nested `{`".into());
        }
        if !is_valid_param_name(name) {
            return Err(format!("invalid placeholder `{{{name}}}`"));
        }
        out.push(name.to_owned());
        rest = &after[close + 1..];
    }
    Ok(out)
}

impl ToolInput {
    pub fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errs = ValidationErrors::default();
        if !is_valid_name(&self.name) {
            errs.push(
                "name",
                format!("must be 1–{MAX_NAME_LEN} characters of a-z, A-Z, 0-9, _ or -"),
            );
        }
        if self.description.chars().count() > MAX_DESCRIPTION_LEN {
            errs.push(
                "description",
                format!("must be at most {MAX_DESCRIPTION_LEN} characters"),
            );
        }
        if matches!(&self.response_shape, Some(s) if s.trim().is_empty()) {
            errs.push("response_shape", "must not be blank; omit it instead");
        }
        if self.tags.len() > MAX_TAGS {
            errs.push("tags", format!("at most {MAX_TAGS} tags"));
        }
        for (i, t) in self.tags.iter().enumerate() {
            if t.trim().is_empty() || t.chars().count() > 64 || t.chars().any(char::is_control) {
                errs.push(format!("tags[{i}]"), "must be 1–64 characters");
            }
        }
        match &self.kind {
            ToolKind::Http(h) => h.validate_into(&mut errs),
        }
        errs.into_result()
    }
}

impl HttpSpec {
    fn validate_into(&self, errs: &mut ValidationErrors) {
        validate_base_url(&self.base_url, errs);

        let placeholders = if !self.path_template.starts_with('/') {
            errs.push("kind.path_template", "must start with `/`");
            None
        } else if self.path_template.contains(['?', '#']) {
            errs.push(
                "kind.path_template",
                "must not contain `?` or `#`; use query params instead",
            );
            None
        } else {
            match path_placeholders(&self.path_template) {
                Ok(p) => Some(p),
                Err(e) => {
                    errs.push("kind.path_template", e);
                    None
                }
            }
        };
        if let Some(p) = &placeholders {
            let mut seen = HashSet::new();
            for name in p {
                if !seen.insert(name) {
                    errs.push(
                        "kind.path_template",
                        format!("placeholder `{{{name}}}` appears more than once"),
                    );
                }
            }
        }

        let mut seen: HashSet<(ParamLocation, &str)> = HashSet::new();
        for (i, p) in self.params.iter().enumerate() {
            let at = |f: &str| format!("kind.params[{i}].{f}");
            if !is_valid_param_name(&p.name) {
                errs.push(
                    at("name"),
                    format!("must be 1–{MAX_NAME_LEN} characters of a-z, A-Z, 0-9, _, . or -"),
                );
            }
            if !seen.insert((p.location, p.wire())) {
                errs.push(
                    at("name"),
                    format!("duplicate {} param `{}`", p.location.as_str(), p.wire()),
                );
            }
            if let Some(w) = &p.wire_name {
                if p.location == ParamLocation::Path {
                    errs.push(
                        at("wire_name"),
                        "path params can't have a wire name; rename the `{placeholder}` instead",
                    );
                } else if w.is_empty() || w.len() > 256 || w.chars().any(char::is_control) {
                    errs.push(
                        at("wire_name"),
                        "must be 1–256 characters without control characters",
                    );
                } else if p.location == ParamLocation::Header
                    && !w
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
                {
                    errs.push(at("wire_name"), "not a valid HTTP header name");
                }
            }
            if !p.schema.is_object() {
                errs.push(at("schema"), "must be a JSON Schema object");
            }
            if p.location == ParamLocation::Path {
                if let Some(ph) = &placeholders {
                    if !ph.contains(&p.name) {
                        errs.push(
                            at("name"),
                            format!(
                                "path param `{}` has no `{{{}}}` in path_template",
                                p.name, p.name
                            ),
                        );
                    }
                }
                if !p.required && p.fixed_value.is_none() {
                    errs.push(
                        at("required"),
                        "path params must be required or have a fixed value",
                    );
                }
            }
        }
        if let Some(ph) = &placeholders {
            for name in ph {
                let declared = self
                    .params
                    .iter()
                    .any(|p| p.location == ParamLocation::Path && &p.name == name);
                if !declared {
                    errs.push(
                        "kind.params",
                        format!("missing path param for `{{{name}}}` in path_template"),
                    );
                }
            }
        }

        if let Some(schema) = &self.body_schema {
            if !schema.is_object() {
                errs.push("kind.body_schema", "must be a JSON Schema object");
            }
            if self
                .params
                .iter()
                .any(|p| p.location == ParamLocation::Body)
            {
                errs.push(
                    "kind.body_schema",
                    "use either body params or body_schema, not both",
                );
            }
        }

        for problem in self.derived_arg_problems() {
            errs.push("kind.params", problem);
        }

        if self.timeout_ms == 0 || self.timeout_ms > MAX_TIMEOUT_MS {
            errs.push(
                "kind.timeout_ms",
                format!("must be between 1 and {MAX_TIMEOUT_MS}"),
            );
        }
    }
}

fn validate_base_url(raw: &str, errs: &mut ValidationErrors) {
    const F: &str = "kind.base_url";
    let url = match url::Url::parse(raw) {
        Ok(u) => u,
        Err(e) => {
            errs.push(F, format!("not a valid URL: {e}"));
            return;
        }
    };
    if !matches!(url.scheme(), "http" | "https") {
        errs.push(F, "scheme must be http or https");
    }
    if url.host_str().is_none_or(str::is_empty) {
        errs.push(F, "must include a host");
    }
    if !url.username().is_empty() || url.password().is_some() {
        errs.push(F, "must not embed credentials; use an auth config");
    }
    if url.query().is_some() || url.fragment().is_some() {
        errs.push(F, "must not contain a query string or fragment");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool(spec: Value) -> ToolInput {
        serde_json::from_value(json!({ "name": "get_user", "kind": spec })).unwrap()
    }

    fn valid_spec() -> Value {
        json!({
            "type": "http",
            "method": "GET",
            "base_url": "https://api.example.com/v1",
            "path_template": "/users/{id}",
            "params": [
                { "name": "id", "location": "path", "required": true },
                { "name": "verbose", "location": "query", "schema": { "type": "boolean" } }
            ]
        })
    }

    fn errs(spec: Value) -> ValidationErrors {
        tool(spec).validate().unwrap_err()
    }

    #[test]
    fn valid_tool_with_defaults() {
        let t = tool(valid_spec());
        t.validate().unwrap();
        assert!(t.enabled);
        let ToolKind::Http(h) = &t.kind;
        assert_eq!(h.timeout_ms, DEFAULT_TIMEOUT_MS);
        assert_eq!(h.params[0].schema, json!({ "type": "string" }));
    }

    #[test]
    fn serde_round_trip() {
        let t = tool(valid_spec());
        let back: ToolInput = serde_json::from_value(serde_json::to_value(&t).unwrap()).unwrap();
        assert_eq!(t, back);
        assert_eq!(
            serde_json::to_value(Source::Manual).unwrap(),
            json!({ "type": "manual" })
        );
    }

    #[test]
    fn rejects_bad_tool_name() {
        let mut t = tool(valid_spec());
        t.name = "get user!".into();
        assert!(t.validate().unwrap_err().has("name"));
    }

    #[test]
    fn placeholder_parsing() {
        assert_eq!(path_placeholders("/a/{x}/b/{y}").unwrap(), ["x", "y"]);
        assert!(path_placeholders("/a/{x").is_err());
        assert!(path_placeholders("/a/x}").is_err());
        assert!(path_placeholders("/a/{}").is_err());
        assert!(path_placeholders("/a/{x{y}}").is_err());
    }

    #[test]
    fn path_params_must_match_template() {
        let mut s = valid_spec();
        s["path_template"] = json!("/users/{user_id}");
        let e = errs(s);
        assert!(e.has("kind.params"), "{e}"); // {user_id} undeclared
        assert!(e.has("kind.params[0].name"), "{e}"); // id not in template
    }

    #[test]
    fn path_params_must_be_required() {
        let mut s = valid_spec();
        s["params"][0]["required"] = json!(false);
        assert!(errs(s.clone()).has("kind.params[0].required"));
        s["params"][0]["fixed_value"] = json!("42");
        tool(s).validate().unwrap();
    }

    #[test]
    fn duplicate_params_per_location() {
        let mut s = valid_spec();
        s["params"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "name": "verbose", "location": "query" }));
        assert!(errs(s.clone()).has("kind.params[2].name"));
        // Same name in a different location is allowed.
        s["params"][2]["location"] = json!("header");
        tool(s).validate().unwrap();
    }

    #[test]
    fn wire_names() {
        let mut s = valid_spec();
        s["params"][1]["name"] = json!("filter_name");
        s["params"][1]["wire_name"] = json!("filter[name]");
        tool(s.clone()).validate().unwrap();
        // Same wire name twice in one location is a duplicate.
        s["params"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "name": "other", "location": "query", "wire_name": "filter[name]" }));
        assert!(errs(s.clone()).has("kind.params[2].name"));
        // Not on path params.
        let mut s = valid_spec();
        s["params"][0]["wire_name"] = json!("user-id");
        assert!(errs(s).has("kind.params[0].wire_name"));
    }

    #[test]
    fn base_url_rules() {
        for bad in [
            "not a url",
            "ftp://x.com",
            "https://user:pw@x.com",
            "https://x.com?a=1",
            "https://x.com#f",
        ] {
            let mut s = valid_spec();
            s["base_url"] = json!(bad);
            assert!(errs(s).has("kind.base_url"), "{bad}");
        }
    }

    #[test]
    fn path_template_rules() {
        for bad in ["users/{id}", "/users/{id}?x=1", "/users/{id}/{id}"] {
            let mut s = valid_spec();
            s["path_template"] = json!(bad);
            assert!(errs(s).has("kind.path_template"), "{bad}");
        }
    }

    #[test]
    fn body_schema_and_body_params_exclusive() {
        let mut s = valid_spec();
        s["method"] = json!("POST");
        s["body_schema"] = json!({ "type": "object" });
        tool(s.clone()).validate().unwrap();
        s["params"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "name": "title", "location": "body" }));
        assert!(errs(s).has("kind.body_schema"));
    }

    #[test]
    fn schema_must_be_object_and_timeout_in_range() {
        let mut s = valid_spec();
        s["params"][1]["schema"] = json!("string");
        s["timeout_ms"] = json!(0);
        let e = errs(s);
        assert!(e.has("kind.params[1].schema") && e.has("kind.timeout_ms"));
    }
}
