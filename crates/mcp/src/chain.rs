//! Running chained tools: each step goes through [`run_tool`], so its
//! argument checks, limits and call-log entry (linked to the chain's call)
//! apply. A failed step stops the chain. With `dry_run`, database writes
//! get `dry_run: true` and other writing steps are skipped.

use std::collections::{HashMap, HashSet};

use forge_core::{
    check_args, step_context, ChainSpec, DbConnection, HttpMethod, ProjectSettings, ToolDef,
    ToolKind, MAX_CHAIN_DEPTH,
};
use forge_exec_http::shape;
use forge_store::{Store, StoreError};
use serde::Serialize;
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::{run_tool, shaped_text, Caller, Executors, RunOutcome, ToolRun};

/// What one step did.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StepTrace {
    pub tool: String,
    pub args: Value,
    pub result: Value,
    pub is_error: bool,
    /// Not run: a write during a dry run that can't be tried safely.
    pub skipped: bool,
}

/// Every step of a chain run, the final output and, on failure, why.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ChainTrace {
    pub steps: Vec<StepTrace>,
    pub output: Option<Value>,
    pub error: Option<String>,
}

/// Whether running `t` can change data (chains: any step, recursively).
pub fn tool_writes(
    t: &ToolDef,
    tools: &HashMap<String, ToolDef>,
    conns: &HashMap<Uuid, DbConnection>,
    depth: usize,
) -> bool {
    match &t.kind {
        ToolKind::Http(h) => !matches!(
            h.method,
            HttpMethod::Get | HttpMethod::Head | HttpMethod::Options
        ),
        ToolKind::Graphql(g) => g.operation == forge_core::GraphqlOperation::Mutation,
        ToolKind::Sql(s) => conns
            .get(&s.connection_id)
            .is_some_and(|c| forge_exec_sql::tool_writes(s, c)),
        ToolKind::Nosql(s) => conns
            .get(&s.connection_id)
            .is_some_and(|c| forge_exec_nosql::tool_writes(s, c)),
        ToolKind::Chain(c) => {
            depth < MAX_CHAIN_DEPTH
                && c.steps.iter().any(|s| {
                    tools
                        .get(&s.tool)
                        .is_some_and(|st| tool_writes(st, tools, conns, depth + 1))
                })
        }
    }
}

/// Whether calling `t` needs a person's approval: its own setting, or any
/// step's (asked once, for the whole chain).
pub fn needs_approval(t: &ToolDef, tools: &HashMap<String, ToolDef>, depth: usize) -> bool {
    t.require_approval
        || match &t.kind {
            ToolKind::Chain(c) if depth < MAX_CHAIN_DEPTH => c.steps.iter().any(|s| {
                tools
                    .get(&s.tool)
                    .is_some_and(|st| needs_approval(st, tools, depth + 1))
            }),
            _ => false,
        }
}

/// Problems with a chain about to be saved as `name`: missing steps, calls
/// to itself, cycles, nesting deeper than allowed, jq that doesn't compile.
pub fn check(name: &str, spec: &ChainSpec, tools: &[ToolDef]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let by_name: HashMap<&str, &ToolDef> = tools.iter().map(|t| (t.name.as_str(), t)).collect();
    for (i, s) in spec.steps.iter().enumerate() {
        if s.tool == name {
            out.push((
                format!("kind.steps[{i}].tool"),
                "a chain can't call itself".into(),
            ));
        } else if !by_name.contains_key(s.tool.as_str()) {
            out.push((
                format!("kind.steps[{i}].tool"),
                format!("there's no tool named `{}`", s.tool),
            ));
        }
        if !s.args.trim().is_empty() {
            if let Err(e) = shape::check(&s.args) {
                out.push((format!("kind.steps[{i}].args"), e));
            }
        }
    }
    if let Some(o) = spec.output.as_deref().filter(|o| !o.trim().is_empty()) {
        if let Err(e) = shape::check(o) {
            out.push(("kind.output".into(), e));
        }
    }
    // Walk the chains this one reaches (with the new version of itself).
    fn steps_of<'a>(
        n: &str,
        name: &str,
        spec: &'a ChainSpec,
        by_name: &HashMap<&str, &'a ToolDef>,
    ) -> Vec<&'a str> {
        if n == name {
            return spec.steps.iter().map(|s| s.tool.as_str()).collect();
        }
        match by_name.get(n).map(|t| &t.kind) {
            Some(ToolKind::Chain(c)) => c.steps.iter().map(|s| s.tool.as_str()).collect(),
            _ => vec![],
        }
    }
    fn walk<'a>(
        n: &'a str,
        name: &str,
        spec: &'a ChainSpec,
        by_name: &HashMap<&str, &'a ToolDef>,
        path: &mut Vec<&'a str>,
        seen: &mut HashSet<&'a str>,
    ) -> Result<usize, String> {
        if path.contains(&n) {
            let mut cycle: Vec<&str> = path.iter().skip_while(|p| **p != n).copied().collect();
            cycle.push(n);
            return Err(format!("steps form a loop: {}", cycle.join(" → ")));
        }
        let next = steps_of(n, name, spec, by_name);
        let is_chain =
            n == name || matches!(by_name.get(n).map(|t| &t.kind), Some(ToolKind::Chain(_)));
        if !is_chain {
            return Ok(0);
        }
        path.push(n);
        let mut depth = 0;
        for s in next {
            if s == n && n == name {
                continue; // reported above
            }
            depth = depth.max(walk(s, name, spec, by_name, path, seen)?);
        }
        path.pop();
        seen.insert(n);
        Ok(depth + 1)
    }
    let mut seen = HashSet::new();
    match walk(name, name, spec, &by_name, &mut Vec::new(), &mut seen) {
        Err(e) => out.push(("kind.steps".into(), e)),
        Ok(d) if d > MAX_CHAIN_DEPTH => out.push((
            "kind.steps".into(),
            format!("chains may call chains at most {MAX_CHAIN_DEPTH} levels deep"),
        )),
        Ok(_) => {}
    }
    out
}

fn failed(trace: ChainTrace, text: String) -> ToolRun {
    ToolRun {
        outcome: RunOutcome::Chain(trace),
        is_error: true,
        text,
        shaped: None,
    }
}

/// Where a chain runs.
pub(crate) struct Env<'a> {
    pub store: &'a Store,
    pub exec: &'a Executors,
    pub settings: &'a ProjectSettings,
}

/// Runs a chain whose own call (already being logged) is `call_id`.
pub(crate) async fn run(
    env: Env<'_>,
    tool: &ToolDef,
    spec: &ChainSpec,
    args: &Map<String, Value>,
    caller: &Caller,
    call_id: Uuid,
) -> Result<ToolRun, StoreError> {
    let Env {
        store,
        exec,
        settings,
    } = env;
    let mut trace = ChainTrace::default();
    if caller.depth >= MAX_CHAIN_DEPTH {
        let e = format!("chains may call chains at most {MAX_CHAIN_DEPTH} levels deep");
        trace.error = Some(e.clone());
        return Ok(failed(trace, e));
    }
    let tools: HashMap<String, ToolDef> = store
        .list_tools(tool.project_id)
        .await?
        .into_iter()
        .map(|t| (t.name.clone(), t))
        .collect();
    let conns: HashMap<Uuid, DbConnection> = store
        .list_connections(tool.project_id)
        .await?
        .into_iter()
        .map(|c| (c.id, c))
        .collect();
    let derived = input_for(tool, spec, &tools, &conns);
    if let Err(errs) = check_args(&derived.bindings, &derived.schema, args) {
        let text = crate::invalid_args_text(&errs);
        trace.error = Some(text.clone());
        return Ok(failed(trace, text));
    }
    let dry_run = args
        .get("dry_run")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut input = args.clone();
    input.remove("dry_run");
    let n = spec.steps.len();
    let mut results: Vec<Value> = Vec::with_capacity(n);
    for (i, step) in spec.steps.iter().enumerate() {
        let label = format!("Step {} of {n} (`{}`)", i + 1, step.tool);
        let Some(t) = tools.get(&step.tool).filter(|t| t.enabled) else {
            let e = format!("{label} can't run: the tool is missing or turned off");
            trace.error = Some(e.clone());
            return Ok(failed(trace, e));
        };
        let ctx = step_context(&input, &results);
        let step_args = if step.args.trim().is_empty() {
            Value::Object(Map::new())
        } else {
            match shape::apply(&step.args, &ctx) {
                Ok(v) => v,
                Err(e) => {
                    let e = format!("{label}: its arguments couldn't be computed: {e}");
                    trace.error = Some(e.clone());
                    return Ok(failed(trace, e));
                }
            }
        };
        let Value::Object(mut a) = step_args else {
            let e = format!("{label}: its arguments must be a JSON object, got {step_args}");
            trace.error = Some(e.clone());
            return Ok(failed(trace, e));
        };
        if dry_run && tool_writes(t, &tools, &conns, caller.depth + 1) {
            match &t.kind {
                ToolKind::Sql(_) | ToolKind::Nosql(_) | ToolKind::Chain(_) => {
                    a.insert("dry_run".into(), Value::Bool(true));
                }
                ToolKind::Http(_) | ToolKind::Graphql(_) => {
                    trace.steps.push(StepTrace {
                        tool: t.name.clone(),
                        args: Value::Object(a),
                        result: Value::Null,
                        is_error: false,
                        skipped: true,
                    });
                    results.push(Value::Null);
                    continue;
                }
            }
        }
        let mut step_caller = caller.clone();
        step_caller.parent = Some(call_id);
        step_caller.depth = caller.depth + 1;
        let run = Box::pin(run_tool(store, exec, t, &a, settings, &step_caller)).await?;
        let value =
            serde_json::from_str(&run.text).unwrap_or_else(|_| Value::String(run.text.clone()));
        trace.steps.push(StepTrace {
            tool: t.name.clone(),
            args: Value::Object(a),
            result: value.clone(),
            is_error: run.is_error,
            skipped: false,
        });
        if run.is_error {
            let e = format!("{label} failed: {}", run.text);
            trace.error = Some(e.clone());
            return Ok(failed(trace, e));
        }
        results.push(value);
    }
    let output = match spec.output.as_deref().filter(|o| !o.trim().is_empty()) {
        Some(expr) => match shape::apply(expr, &json!({ "input": input, "steps": results })) {
            Ok(v) => v,
            Err(e) => {
                let e = format!("The chain's output couldn't be computed: {e}");
                trace.error = Some(e.clone());
                return Ok(failed(trace, e));
            }
        },
        None => results.last().cloned().unwrap_or(Value::Null),
    };
    trace.output = Some(output.clone());
    let body = match &output {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let (text, changed) = shaped_text(&body, Some("application/json"), tool, settings);
    Ok(ToolRun {
        outcome: RunOutcome::Chain(trace),
        is_error: false,
        shaped: changed.then(|| text.clone()),
        text,
    })
}

/// The chain's input: its own arguments, plus `dry_run` when a step writes.
pub fn input_for(
    tool: &ToolDef,
    spec: &ChainSpec,
    tools: &HashMap<String, ToolDef>,
    conns: &HashMap<Uuid, DbConnection>,
) -> forge_core::DerivedInput {
    let mut d = spec.derive_input();
    if tool_writes(tool, tools, conns, 0) {
        if let Some(props) = d
            .schema
            .get_mut("properties")
            .and_then(Value::as_object_mut)
        {
            props.insert(
                "dry_run".into(),
                json!({ "type": "boolean",
                    "description": "If true, database steps only report what they would change, and other steps that change data are skipped." }),
            );
        }
        d.bindings.push(forge_core::ArgBinding {
            arg: "dry_run".into(),
            target: forge_core::ArgTarget::SqlParam {
                name: "dry_run".into(),
            },
            required: false,
        });
    }
    d
}
