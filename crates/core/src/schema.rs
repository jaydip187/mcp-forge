//! Derives a tool's MCP input schema from its IR, plus the map that routes
//! each agent-facing argument back to its place in the request.

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
    /// A SQL tool's `:name` placeholder.
    SqlParam { name: String },
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
#[path = "../tests/unit/schema.rs"]
mod tests;
