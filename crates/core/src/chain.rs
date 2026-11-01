//! Chained tools: steps that call other tools of the project in order. Each
//! step's arguments come from a jq expression over
//! `{input, steps, last}` (the chain's arguments, every earlier step's
//! result, and the previous one); `output` (jq over `{input, steps}`)
//! shapes the result, which is otherwise the last step's.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::db::{is_valid_sql_param_name, SqlParam, SQL_PARAM_TYPES};
use crate::schema::{ArgBinding, ArgTarget, DerivedInput};
use crate::validate::{is_valid_name, ValidationErrors};

pub const MAX_CHAIN_STEPS: usize = 10;
/// A chain may call chains, this many levels deep.
pub const MAX_CHAIN_DEPTH: usize = 3;
const MAX_EXPR_LEN: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChainStep {
    /// Name of the tool to call.
    pub tool: String,
    /// jq producing the step's arguments (an object); empty means `{}`.
    #[serde(default)]
    pub args: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChainSpec {
    /// The chain's own arguments.
    #[serde(default)]
    pub params: Vec<SqlParam>,
    pub steps: Vec<ChainStep>,
    /// jq over `{input, steps}`; the last step's result when absent.
    #[serde(default)]
    pub output: Option<String>,
}

/// What a step's jq expression sees.
pub fn step_context(input: &Map<String, Value>, results: &[Value]) -> Value {
    json!({
        "input": input,
        "steps": results,
        "last": results.last().cloned().unwrap_or(Value::Null),
    })
}

impl ChainSpec {
    pub fn derive_input(&self) -> DerivedInput {
        let mut properties = Map::new();
        let mut required = Vec::new();
        let mut bindings = Vec::new();
        for p in &self.params {
            let mut schema = p.schema.clone();
            if let (Some(desc), Some(obj)) = (&p.description, schema.as_object_mut()) {
                obj.entry("description")
                    .or_insert_with(|| Value::String(desc.clone()));
            }
            properties.insert(p.name.clone(), schema);
            if p.required {
                required.push(Value::String(p.name.clone()));
            }
            bindings.push(ArgBinding {
                arg: p.name.clone(),
                target: ArgTarget::SqlParam {
                    name: p.name.clone(),
                },
                required: p.required,
            });
        }
        let Value::Object(schema) = json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        }) else {
            unreachable!("json! object literal")
        };
        DerivedInput { schema, bindings }
    }

    /// Structural checks; step tools, cycles and jq compile in the server.
    pub(crate) fn validate_into(&self, errs: &mut ValidationErrors) {
        if self.steps.is_empty() {
            errs.push("kind.steps", "add at least one step");
        }
        if self.steps.len() > MAX_CHAIN_STEPS {
            errs.push("kind.steps", format!("at most {MAX_CHAIN_STEPS} steps"));
        }
        for (i, s) in self.steps.iter().enumerate() {
            if !is_valid_name(&s.tool) {
                errs.push(format!("kind.steps[{i}].tool"), "pick a tool");
            }
            if s.args.len() > MAX_EXPR_LEN {
                errs.push(
                    format!("kind.steps[{i}].args"),
                    format!("at most {MAX_EXPR_LEN} characters"),
                );
            }
        }
        if self.output.as_ref().is_some_and(|o| o.len() > MAX_EXPR_LEN) {
            errs.push("kind.output", format!("at most {MAX_EXPR_LEN} characters"));
        }
        let mut seen = HashSet::new();
        for (i, p) in self.params.iter().enumerate() {
            let at = |f: &str| format!("kind.params[{i}].{f}");
            if !is_valid_sql_param_name(&p.name) {
                errs.push(
                    at("name"),
                    "must start with a letter or _ and contain only a-z, A-Z, 0-9 or _",
                );
            }
            if !seen.insert(p.name.to_lowercase()) {
                errs.push(at("name"), format!("duplicate input `{}`", p.name));
            }
            if p.name == "dry_run" {
                errs.push(
                    at("name"),
                    "`dry_run` is added automatically when the chain writes",
                );
            }
            match p.schema.get("type").and_then(Value::as_str) {
                _ if !p.schema.is_object() => {
                    errs.push(at("schema"), "must be a JSON Schema object")
                }
                Some(t) if SQL_PARAM_TYPES.contains(&t) => {}
                None => {}
                Some(t) => errs.push(
                    at("schema"),
                    format!("type `{t}` is not supported; use string, integer, number or boolean"),
                ),
            }
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/chain.rs"]
mod tests;
