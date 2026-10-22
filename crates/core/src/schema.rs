//! Derives a tool's MCP input schema from its IR, plus the map that routes
//! each agent-facing argument back to its place in the HTTP request.

use std::collections::HashMap;

use serde_json::{json, Map, Value};

use crate::tool::{is_valid_param_name, HttpSpec, ParamLocation};

/// Agent-facing argument name for a whole-request `body_schema`.
pub const BODY_ARG: &str = "body";

/// Where an argument's value goes in the outgoing request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgTarget {
    /// A declared param. `name` is its IR name (the `{placeholder}` for path
    /// params); `wire` is the name sent upstream.
    Param {
        location: ParamLocation,
        name: String,
        wire: String,
    },
    /// The entire JSON request body (`body_schema`).
    WholeBody,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArgBinding {
    /// Name the agent uses (input-schema property key).
    pub arg: String,
    pub target: ArgTarget,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DerivedInput {
    /// JSON Schema object for MCP `inputSchema`.
    pub schema: Map<String, Value>,
    pub bindings: Vec<ArgBinding>,
}

impl ParamLocation {
    pub fn as_str(self) -> &'static str {
        match self {
            ParamLocation::Path => "path",
            ParamLocation::Query => "query",
            ParamLocation::Header => "header",
            ParamLocation::Body => "body",
        }
    }
}

struct Exposed<'a> {
    base: &'a str,
    location: ParamLocation,
    target: ArgTarget,
    schema: &'a Value,
    description: Option<&'a str>,
    required: bool,
}

impl HttpSpec {
    /// Builds the input schema. Params with a `fixed_value` are hidden. A name
    /// used in more than one location is prefixed with its location
    /// (`path_id`, `query_id`); unique names are left as-is.
    pub fn derive_input(&self) -> DerivedInput {
        let mut exposed: Vec<Exposed> = self
            .params
            .iter()
            .filter(|p| p.fixed_value.is_none())
            .map(|p| Exposed {
                base: &p.name,
                location: p.location,
                target: ArgTarget::Param {
                    location: p.location,
                    name: p.name.clone(),
                    wire: p.wire().to_owned(),
                },
                schema: &p.schema,
                description: p.description.as_deref(),
                required: p.required,
            })
            .collect();
        if let Some(schema) = &self.body_schema {
            exposed.push(Exposed {
                base: BODY_ARG,
                location: ParamLocation::Body,
                target: ArgTarget::WholeBody,
                schema,
                description: Some("Request body (JSON)."),
                required: true,
            });
        }

        let mut counts: HashMap<&str, usize> = HashMap::new();
        for e in &exposed {
            *counts.entry(e.base).or_default() += 1;
        }

        let mut properties = Map::new();
        let mut required = Vec::new();
        let mut bindings = Vec::new();
        for e in exposed {
            let arg = if counts[e.base] > 1 {
                format!("{}_{}", e.location.as_str(), e.base)
            } else {
                e.base.to_owned()
            };
            let mut schema = e.schema.clone();
            if let (Some(desc), Some(obj)) = (e.description, schema.as_object_mut()) {
                obj.entry("description")
                    .or_insert_with(|| Value::String(desc.to_owned()));
            }
            properties.insert(arg.clone(), schema);
            if e.required {
                required.push(Value::String(arg.clone()));
            }
            bindings.push(ArgBinding {
                arg,
                target: e.target,
                required: e.required,
            });
        }

        let schema = json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        });
        let Value::Object(schema) = schema else {
            unreachable!("json! object literal")
        };
        DerivedInput { schema, bindings }
    }

    /// Problems with the derived argument names (after collision prefixing).
    pub(crate) fn derived_arg_problems(&self) -> Vec<String> {
        let derived = self.derive_input();
        let mut seen = HashMap::new();
        let mut problems = Vec::new();
        for b in &derived.bindings {
            if !is_valid_param_name(&b.arg) {
                problems.push(format!(
                    "derived argument name `{}` is not MCP-safe (max 64 of a-z, A-Z, 0-9, _, . or -); rename the param",
                    b.arg
                ));
            }
            if seen.insert(b.arg.as_str(), ()).is_some() {
                problems.push(format!(
                    "two params map to the same argument name `{}`; rename one",
                    b.arg
                ));
            }
        }
        problems
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::ToolInput;

    fn spec(v: Value) -> HttpSpec {
        let t: ToolInput = serde_json::from_value(json!({ "name": "t", "kind": v })).unwrap();
        let crate::ToolKind::Http(h) = t.kind;
        h
    }

    #[test]
    fn derives_schema_and_hides_fixed() {
        let s = spec(json!({
            "type": "http", "method": "GET", "base_url": "https://x.io", "path_template": "/u/{id}",
            "params": [
                { "name": "id", "location": "path", "required": true, "schema": { "type": "integer" }, "description": "User id" },
                { "name": "q", "location": "query" },
                { "name": "X-Version", "location": "header", "fixed_value": "2" }
            ]
        }));
        let d = s.derive_input();
        assert_eq!(
            Value::Object(d.schema),
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "integer", "description": "User id" },
                    "q": { "type": "string" }
                },
                "required": ["id"],
                "additionalProperties": false
            })
        );
        assert_eq!(d.bindings.len(), 2);
        assert_eq!(
            d.bindings[0].target,
            ArgTarget::Param {
                location: ParamLocation::Path,
                name: "id".into(),
                wire: "id".into(),
            }
        );
    }

    #[test]
    fn prefixes_only_on_collision() {
        let s = spec(json!({
            "type": "http", "method": "PUT", "base_url": "https://x.io", "path_template": "/u/{id}",
            "params": [
                { "name": "id", "location": "path", "required": true },
                { "name": "id", "location": "body" },
                { "name": "name", "location": "body" }
            ]
        }));
        let args: Vec<_> = s
            .derive_input()
            .bindings
            .into_iter()
            .map(|b| b.arg)
            .collect();
        assert_eq!(args, ["path_id", "body_id", "name"]);
    }

    #[test]
    fn whole_body_arg() {
        let s = spec(json!({
            "type": "http", "method": "POST", "base_url": "https://x.io", "path_template": "/u",
            "body_schema": { "type": "object", "properties": { "a": { "type": "string" } } }
        }));
        let d = s.derive_input();
        assert_eq!(d.bindings[0].arg, BODY_ARG);
        assert_eq!(d.bindings[0].target, ArgTarget::WholeBody);
        assert_eq!(d.schema["required"], json!(["body"]));
    }

    #[test]
    fn validation_catches_prefix_clash() {
        // `query_id` (literal) clashes with the prefixed form of `id` in query.
        let t: ToolInput = serde_json::from_value(json!({ "name": "t", "kind": {
            "type": "http", "method": "GET", "base_url": "https://x.io", "path_template": "/u/{id}",
            "params": [
                { "name": "id", "location": "path", "required": true },
                { "name": "id", "location": "query" },
                { "name": "query_id", "location": "header" }
            ]
        }}))
        .unwrap();
        assert!(t.validate().unwrap_err().has("kind.params"));
    }
}
