//! GraphQL tools: one query or mutation document against an endpoint, with
//! the operation's variables as the agent's arguments.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::schema::{ArgBinding, ArgTarget, DerivedInput};
use crate::validate::ValidationErrors;

const MAX_DOCUMENT_LEN: usize = 100_000;
const DEFAULT_TIMEOUT_MS: u32 = 30_000;

fn default_timeout() -> u32 {
    DEFAULT_TIMEOUT_MS
}

/// Whether the document reads (`query`) or changes (`mutation`) data.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphqlOperation {
    #[default]
    Query,
    Mutation,
}

impl GraphqlOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            GraphqlOperation::Query => "query",
            GraphqlOperation::Mutation => "mutation",
        }
    }
}

/// A variable of the operation, exposed to the agent as an argument.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphqlVar {
    /// Without the `$`.
    pub name: String,
    /// GraphQL type as written in the document, e.g. `ID!` or `[String!]`.
    pub graphql_type: String,
    #[serde(default)]
    pub description: String,
    /// JSON Schema for the argument; derived from `graphql_type` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<Value>,
}

impl GraphqlVar {
    pub fn required(&self) -> bool {
        self.graphql_type.trim_end().ends_with('!')
    }

    /// The argument's JSON Schema.
    pub fn json_schema(&self) -> Value {
        let mut s = self
            .schema
            .clone()
            .unwrap_or_else(|| schema_for_type(&self.graphql_type));
        if !self.description.is_empty() {
            if let Some(o) = s.as_object_mut() {
                o.insert("description".into(), json!(self.description));
            }
        }
        s
    }
}

/// JSON Schema for a GraphQL type reference like `[Int!]!` (built-in
/// scalars only; anything else accepts any JSON).
pub fn schema_for_type(t: &str) -> Value {
    let t = t.trim().trim_end_matches('!');
    if let Some(inner) = t.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
        return json!({ "type": "array", "items": schema_for_type(inner) });
    }
    match t {
        "Int" => json!({ "type": "integer" }),
        "Float" => json!({ "type": "number" }),
        "String" | "ID" => json!({ "type": "string" }),
        "Boolean" => json!({ "type": "boolean" }),
        _ => json!({}),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphqlSpec {
    /// The GraphQL endpoint, e.g. `https://api.example.com/graphql`.
    pub endpoint: String,
    pub operation: GraphqlOperation,
    /// The query or mutation document sent as `query`.
    pub document: String,
    #[serde(default)]
    pub operation_name: Option<String>,
    #[serde(default)]
    pub variables: Vec<GraphqlVar>,
    #[serde(default)]
    pub auth_ref: Option<Uuid>,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u32,
}

fn valid_var_name(n: &str) -> bool {
    let mut c = n.chars();
    c.next()
        .is_some_and(|f| f.is_ascii_alphabetic() || f == '_')
        && c.all(|x| x.is_ascii_alphanumeric() || x == '_')
        && n.len() <= 64
}

fn valid_type(t: &str) -> bool {
    let core: String = t
        .chars()
        .filter(|c| !matches!(c, '[' | ']' | '!'))
        .collect();
    valid_var_name(&core)
        && t.chars().filter(|&c| c == '[').count() == t.chars().filter(|&c| c == ']').count()
}

/// How often `$name` appears as a whole variable (`$id` doesn't count `$idx`).
fn var_mentions(doc: &str, name: &str) -> usize {
    let needle = format!("${name}");
    doc.match_indices(&needle)
        .filter(|(i, _)| {
            !doc[i + needle.len()..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        })
        .count()
}

impl GraphqlSpec {
    pub fn derive_input(&self) -> DerivedInput {
        let mut props = Map::new();
        let mut required = vec![];
        let mut bindings = vec![];
        for v in &self.variables {
            props.insert(v.name.clone(), v.json_schema());
            if v.required() {
                required.push(v.name.clone());
            }
            bindings.push(ArgBinding {
                arg: v.name.clone(),
                target: ArgTarget::GraphqlVar {
                    name: v.name.clone(),
                },
                required: v.required(),
            });
        }
        let Value::Object(schema) = json!({
            "type": "object",
            "properties": props,
            "required": required,
            "additionalProperties": false,
        }) else {
            unreachable!("json! object literal")
        };
        DerivedInput { schema, bindings }
    }

    pub(crate) fn validate_into(&self, errs: &mut ValidationErrors) {
        match url::Url::parse(&self.endpoint) {
            Ok(u)
                if matches!(u.scheme(), "http" | "https")
                    && u.has_host()
                    && u.username().is_empty()
                    && u.password().is_none()
                    && u.query().is_none()
                    && u.fragment().is_none() => {}
            _ => errs.push(
                "kind.endpoint",
                "must be an http(s) URL without credentials, query or #fragment",
            ),
        }
        let doc = self.document.trim();
        if doc.is_empty() || doc.len() > MAX_DOCUMENT_LEN {
            errs.push(
                "kind.document",
                format!("must be 1–{MAX_DOCUMENT_LEN} characters"),
            );
        } else {
            let first = doc
                .split(|c: char| c.is_whitespace() || c == '(' || c == '{')
                .next()
                .unwrap_or("");
            let ok = match self.operation {
                GraphqlOperation::Query => first == "query" || doc.starts_with('{'),
                GraphqlOperation::Mutation => first == "mutation",
            };
            if !ok {
                errs.push(
                    "kind.document",
                    format!("must start with `{}`", self.operation.as_str()),
                );
            }
        }
        if self.timeout_ms == 0 || self.timeout_ms > 300_000 {
            errs.push("kind.timeout_ms", "must be between 1 and 300000");
        }
        let mut seen = std::collections::HashSet::new();
        for (i, v) in self.variables.iter().enumerate() {
            if !valid_var_name(&v.name) {
                errs.push(
                    format!("kind.variables[{i}].name"),
                    "must be a GraphQL name (letters, digits, _)",
                );
            } else if !seen.insert(v.name.as_str()) {
                errs.push(format!("kind.variables[{i}].name"), "is declared twice");
            } else if var_mentions(doc, &v.name) < 2 {
                // Once in the declaration, at least once in the body.
                errs.push(
                    format!("kind.variables[{i}].name"),
                    format!("`${}` isn't used in the document", v.name),
                );
            }
            if !valid_type(&v.graphql_type) {
                errs.push(
                    format!("kind.variables[{i}].graphql_type"),
                    "must be a GraphQL type like `ID!` or `[String]`",
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/graphql.rs"]
mod tests;
