//! `rmcp` server handler backed by the store: one handler serves one project.
//!
//! Tool definitions are read from the store on every request, so edits are
//! visible immediately. Tool-list changes are announced to connected clients
//! through a [`ChangeBus`]: as `notifications/tools/list_changed` for
//! initialize-based sessions and through `subscriptions/listen` for
//! 2026-07-28 clients.

pub mod approvals;
pub mod oauth;

use std::sync::Arc;
use std::time::{Duration, Instant};

use std::collections::HashMap;

use forge_core::{
    logged_arguments, logged_error, ApprovalDecision, ApprovalRecord, AuditEntry, CallEntry,
    CallStatus, DbConnection, HttpMethod, ProjectSettings, SecretResolver, SqlMode, SqlSpec,
    ToolDef, ToolKind,
};
use forge_exec_http::shape::{shape_body, ShapeOptions};
use forge_exec_http::{ExecError, HttpExecutor, HttpOutcome};
use forge_exec_sql::{SqlError, SqlExecutor, SqlOutcome};
use forge_store::{Store, StoreError};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ClientResult, ContentBlock,
    ElicitRequest, ElicitRequestParams, ElicitationAction, ElicitationSchema, Implementation,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, ServerRequest,
    SubscriptionFilter, Tool, ToolAnnotations,
};
use rmcp::service::{
    ElicitationMode, NotificationContext, Peer, PeerRequestOptions, RequestContext,
    SubscriptionContext,
};
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde_json::{json, Map, Value};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Upstream error bodies are cut to this many characters in tool results.
pub const MAX_ERROR_BODY_CHARS: usize = 4_000;

/// Broadcasts "tools of project X changed" to every live MCP session.
#[derive(Debug, Clone)]
pub struct ChangeBus {
    tx: broadcast::Sender<Uuid>,
}

impl Default for ChangeBus {
    fn default() -> Self {
        Self::new()
    }
}

impl ChangeBus {
    pub fn new() -> Self {
        Self {
            tx: broadcast::channel(64).0,
        }
    }

    pub fn notify(&self, project_id: Uuid) {
        // No receivers just means nobody is connected.
        let _ = self.tx.send(project_id);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Uuid> {
        self.tx.subscribe()
    }
}

/// The executors tool calls run on: HTTP upstreams and SQL databases.
#[derive(Clone)]
pub struct Executors {
    pub http: HttpExecutor,
    pub sql: SqlExecutor,
    /// Calls waiting for a person's approval, shared with the web UI.
    pub approvals: approvals::Approvals,
}

impl Executors {
    pub fn new(secrets: Arc<dyn SecretResolver>) -> Self {
        Self {
            http: HttpExecutor::new(secrets.clone()),
            sql: SqlExecutor::new(secrets),
            approvals: approvals::Approvals::default(),
        }
    }
}

/// Meta-tools listed instead of the real tools when a project has
/// `tool_search` on: agents find tools by keyword, then call them by name.
pub const SEARCH_TOOL: &str = "search_tools";
pub const CALL_TOOL: &str = "call_tool";
const DEFAULT_SEARCH_LIMIT: usize = 8;
const MAX_SEARCH_LIMIT: usize = 25;

/// MCP handler for a single project.
#[derive(Clone)]
pub struct ForgeMcp {
    store: Store,
    project_id: Uuid,
    exec: Executors,
    changes: ChangeBus,
}

impl ForgeMcp {
    pub fn new(store: Store, project_id: Uuid, exec: Executors, changes: ChangeBus) -> Self {
        Self {
            store,
            project_id,
            exec,
            changes,
        }
    }

    async fn settings(&self) -> Result<ProjectSettings, ErrorData> {
        Ok(self
            .store
            .get_project(self.project_id)
            .await
            .map_err(internal)?
            .settings)
    }

    /// Enabled tools that pass the project's filters.
    async fn exposed_tools(&self, settings: &ProjectSettings) -> Result<Vec<ToolDef>, ErrorData> {
        let tools = self
            .store
            .list_tools(self.project_id)
            .await
            .map_err(internal)?;
        let conns = self.connections().await?;
        Ok(tools
            .into_iter()
            .filter(|t| exposed(settings, t, &conns))
            .collect())
    }

    /// The project's DB connections by id (SQL tool descriptions use them).
    async fn connections(&self) -> Result<HashMap<Uuid, DbConnection>, ErrorData> {
        Ok(self
            .store
            .list_connections(self.project_id)
            .await
            .map_err(internal)?
            .into_iter()
            .map(|c| (c.id, c))
            .collect())
    }

    async fn call_named(
        &self,
        name: &str,
        args: &Map<String, Value>,
        settings: &ProjectSettings,
        caller: &Caller,
        peer: &Peer<RoleServer>,
        ct: &CancellationToken,
    ) -> Result<CallToolResult, ErrorData> {
        let conns = self.connections().await?;
        let tool = match self.store.get_tool_by_name(self.project_id, name).await {
            Ok(t) if exposed(settings, &t, &conns) => t,
            Ok(_) | Err(StoreError::NotFound(_)) => {
                let entry = CallEntry {
                    id: Uuid::new_v4(),
                    project_id: self.project_id,
                    ts: chrono::Utc::now(),
                    tool_id: None,
                    tool_name: name.to_owned(),
                    origin: caller.origin.to_owned(),
                    client: caller.client.clone(),
                    arguments: logged_arguments(args),
                    status: CallStatus::UnknownTool,
                    duration_ms: 0,
                    response_chars: 0,
                    upstream_status: None,
                    error: Some(format!("unknown tool `{name}`")),
                    approval: None,
                };
                log_call(&self.store, settings, &entry).await;
                return Err(ErrorData::invalid_params(
                    format!("unknown tool `{name}`"),
                    None,
                ));
            }
            Err(e) => return Err(internal(e)),
        };
        let mut caller = caller.clone();
        if tool.require_approval {
            match self
                .await_approval(&tool, args, settings, &caller, peer, ct)
                .await
            {
                Ok(record) => caller.approval = Some(record),
                Err((record, text)) => {
                    let entry = CallEntry {
                        id: Uuid::new_v4(),
                        project_id: self.project_id,
                        ts: chrono::Utc::now(),
                        tool_id: Some(tool.id),
                        tool_name: tool.name.clone(),
                        origin: caller.origin.to_owned(),
                        client: caller.client.clone(),
                        arguments: logged_arguments(args),
                        status: CallStatus::NotApproved,
                        duration_ms: record.wait_ms,
                        response_chars: text.chars().count() as u64,
                        upstream_status: None,
                        error: Some(logged_error(&text)),
                        approval: Some(record),
                    };
                    log_call(&self.store, settings, &entry).await;
                    return Ok(result(true, text));
                }
            }
        }
        let run = run_tool(&self.store, &self.exec, &tool, args, settings, &caller)
            .await
            .map_err(internal)?;
        tracing::info!(
            tool = name,
            kind = tool.kind.type_name(),
            ok = !run.is_error,
            status = match &run.outcome {
                RunOutcome::Http(Ok(o)) => Some(o.status),
                _ => None,
            },
            "tool call"
        );
        Ok(result(run.is_error, run.text))
    }

    /// Waits for a person to approve a call: asks the client (form
    /// elicitation) and lists it in the web UI; the first answer wins.
    /// `Ok` when approved, else the record and the text for the agent.
    async fn await_approval(
        &self,
        tool: &ToolDef,
        args: &Map<String, Value>,
        settings: &ProjectSettings,
        caller: &Caller,
        peer: &Peer<RoleServer>,
        ct: &CancellationToken,
    ) -> Result<ApprovalRecord, (ApprovalRecord, String)> {
        use approvals::{Decision, PendingApproval};
        let started = Instant::now();
        let approvals = &self.exec.approvals;
        let can_elicit = peer
            .supported_elicitation_modes()
            .contains(&ElicitationMode::Form);
        if !approvals.has_ui() && !can_elicit {
            let record = ApprovalRecord {
                decision: ApprovalDecision::Cancelled,
                decided_by: None,
                wait_ms: 0,
            };
            return Err((record, approvals::NO_WAY_TO_APPROVE.into()));
        }
        let id = Uuid::new_v4();
        let now = chrono::Utc::now();
        let timeout_secs = settings.approval_timeout_secs;
        let arguments = logged_arguments(args);
        let mut rx = approvals.open(PendingApproval {
            id,
            project_id: self.project_id,
            tool_id: tool.id,
            tool_name: tool.name.clone(),
            client: caller.client.clone(),
            arguments: arguments.clone(),
            created_at: now,
            expires_at: now + chrono::Duration::seconds(i64::from(timeout_secs)),
            asked_client: can_elicit,
        });
        // Older protocols only: 2026-07-28 forbids server-to-client requests,
        // so those clients get an error here and rely on the web UI.
        let mut handle = if can_elicit {
            let params = ElicitRequestParams::FormElicitationParams {
                meta: None,
                message: format!(
                    "mcp-forge: allow the tool `{}` to run with these arguments?\n{}\n\n\
                     Accept to run it, decline to refuse.",
                    tool.name,
                    serde_json::to_string_pretty(&arguments).expect("JSON serializes")
                ),
                requested_schema: confirm_schema(),
            };
            match peer
                .send_cancellable_request(
                    ServerRequest::ElicitRequest(ElicitRequest::new(params)),
                    PeerRequestOptions::no_options(),
                )
                .await
            {
                Ok(h) => Some(h),
                Err(e) => {
                    tracing::warn!(error = %e, "could not ask the client for approval");
                    None
                }
            }
        } else {
            None
        };
        let client_label = caller.client.clone().or_else(|| Some("MCP client".into()));
        let mut elicit_open = handle.is_some();
        let sleep = tokio::time::sleep(Duration::from_secs(u64::from(timeout_secs)));
        tokio::pin!(sleep);
        let decision = loop {
            tokio::select! {
                biased;
                d = &mut rx => break d.unwrap_or(Decision {
                    decision: ApprovalDecision::Cancelled,
                    by: None,
                }),
                answer = async {
                    match handle.as_mut() {
                        Some(h) => (&mut h.rx).await,
                        None => std::future::pending().await,
                    }
                }, if elicit_open => {
                    elicit_open = false;
                    let decision = match answer {
                        Ok(Ok(ClientResult::ElicitResult(r))) => Some(match r.action {
                            ElicitationAction::Accept => ApprovalDecision::Approved,
                            _ => ApprovalDecision::Denied,
                        }),
                        // The client failed to ask; keep waiting for the web UI.
                        _ => (!approvals.has_ui()).then_some(ApprovalDecision::Cancelled),
                    };
                    if let Some(d) = decision {
                        approvals.decide(id, d, client_label.clone());
                    }
                }
                () = &mut sleep => {
                    approvals.decide(id, ApprovalDecision::TimedOut, None);
                }
                () = ct.cancelled() => {
                    approvals.decide(id, ApprovalDecision::Cancelled, None);
                }
            }
        };
        if let Some(h) = handle.filter(|_| elicit_open) {
            // Answered elsewhere: close the client's prompt.
            let _ = h.cancel(Some("answered in mcp-forge".into())).await;
        }
        let record = ApprovalRecord {
            decision: decision.decision,
            decided_by: decision.by.clone(),
            wait_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        };
        tracing::info!(
            tool = %tool.name,
            decision = record.decision.as_str(),
            by = record.decided_by.as_deref().unwrap_or("-"),
            "approval"
        );
        if record.decision == ApprovalDecision::Approved {
            Ok(record)
        } else {
            let text = approvals::not_approved_text(&decision, timeout_secs);
            Err((record, text))
        }
    }

    async fn search(
        &self,
        args: &Map<String, Value>,
        settings: &ProjectSettings,
    ) -> Result<CallToolResult, ErrorData> {
        let Some(query) = args.get("query").and_then(Value::as_str) else {
            return Ok(result(
                true,
                "Invalid arguments:\n- `query`: required argument is missing".into(),
            ));
        };
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .map_or(DEFAULT_SEARCH_LIMIT, |l| {
                (l as usize).clamp(1, MAX_SEARCH_LIMIT)
            });
        let tools = self.exposed_tools(settings).await?;
        let conns = self.connections().await?;
        let hits = search_tools(&tools, query, limit);
        if hits.is_empty() {
            return Ok(result(
                false,
                format!(
                    "No tools match `{query}`. Try other words, or an empty query to list some tools."
                ),
            ));
        }
        let found: Vec<Value> = hits
            .iter()
            .map(|t| {
                let m = to_mcp_tool(t, connection_of(t, &conns));
                json!({ "name": m.name, "description": m.description, "input_schema": m.input_schema })
            })
            .collect();
        Ok(result(
            false,
            serde_json::to_string(&found).expect("JSON serializes"),
        ))
    }
}

fn result(is_error: bool, text: String) -> CallToolResult {
    let content = vec![ContentBlock::text(text)];
    if is_error {
        CallToolResult::error(content)
    } else {
        CallToolResult::success(content)
    }
}

/// Ranks tools against a keyword query (name > tags > path > description).
/// An empty query returns the first `limit` tools.
pub fn search_tools<'a>(tools: &'a [ToolDef], query: &str, limit: usize) -> Vec<&'a ToolDef> {
    let terms: Vec<String> = query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect();
    if terms.is_empty() {
        return tools.iter().take(limit).collect();
    }
    let mut scored: Vec<(u32, &ToolDef)> = tools
        .iter()
        .filter_map(|t| {
            let name = t.name.to_lowercase();
            let desc = t.description.to_lowercase();
            let path = match &t.kind {
                ToolKind::Http(h) => h.path_template.to_lowercase(),
                ToolKind::Sql(_) => String::new(),
            };
            let score: u32 = terms
                .iter()
                .map(|term| {
                    let term = term.as_str();
                    let mut s = 0;
                    if name.contains(term) {
                        s += 5;
                    }
                    if t.tags.iter().any(|tag| tag.to_lowercase().contains(term)) {
                        s += 3;
                    }
                    if path.contains(term) {
                        s += 2;
                    }
                    if desc.contains(term) {
                        s += 1;
                    }
                    s
                })
                .sum();
            (score > 0).then_some((score, t))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
    scored.into_iter().take(limit).map(|(_, t)| t).collect()
}

fn search_tool_defs() -> Vec<Tool> {
    let search = Tool::new(
        SEARCH_TOOL,
        "Find tools in this server by keywords (e.g. \"create issue\"). Returns matching tool names, \
         descriptions and input schemas. Call one with call_tool.",
        json_object(json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Words describing what you want to do." },
                "limit": {
                    "type": "integer", "minimum": 1, "maximum": MAX_SEARCH_LIMIT,
                    "description": "Most results to return (default 8)."
                }
            },
            "required": ["query"],
            "additionalProperties": false
        })),
    )
    .with_annotations(ToolAnnotations::new().read_only(true).open_world(false));
    let call = Tool::new(
        CALL_TOOL,
        "Call a tool found with search_tools, passing arguments as its input schema describes.",
        json_object(json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Tool name from search_tools." },
                "arguments": { "type": "object", "description": "Arguments for that tool." }
            },
            "required": ["name"],
            "additionalProperties": false
        })),
    )
    .with_annotations(ToolAnnotations::new().open_world(true));
    vec![search, call]
}

fn json_object(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(m) => m,
        _ => unreachable!("schema literal is an object"),
    }
}

/// What the executor returned, before it became agent text.
#[derive(Debug)]
pub enum RunOutcome {
    Http(Result<HttpOutcome, ExecError>),
    Sql(Result<SqlOutcome, SqlError>),
}

/// Result of running a tool: the raw outcome plus exactly what the agent sees.
#[derive(Debug)]
pub struct ToolRun {
    pub outcome: RunOutcome,
    pub is_error: bool,
    pub text: String,
    /// Set when shaping (jq, null stripping or the size cap) changed the body.
    pub shaped: Option<String>,
}

/// Whether agents can see a tool: the project's filters, plus read-only
/// projects hide SQL tools that write.
fn exposed(settings: &ProjectSettings, t: &ToolDef, conns: &HashMap<Uuid, DbConnection>) -> bool {
    if !settings.exposes(t) {
        return false;
    }
    match (&t.kind, connection_of(t, conns)) {
        (ToolKind::Sql(spec), Some(c)) if settings.read_only => {
            !forge_exec_sql::tool_writes(spec, c)
        }
        _ => true,
    }
}

/// Who made a call: recorded in the call log and the write audit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller {
    /// `mcp` (an agent) or `console` (the UI test console).
    pub origin: &'static str,
    /// MCP client as `name version`, or the signed-in user.
    pub client: Option<String>,
    /// Set once a person approved the call.
    pub approval: Option<ApprovalRecord>,
}

impl Caller {
    pub fn mcp(client: Option<String>) -> Self {
        Self {
            origin: "mcp",
            client,
            approval: None,
        }
    }

    pub fn console(user: Option<String>) -> Self {
        Self {
            origin: "console",
            client: user,
            approval: None,
        }
    }
}

/// Schema for an approval prompt: nothing to fill in, the client's
/// Accept/Decline is the answer.
fn confirm_schema() -> ElicitationSchema {
    ElicitationSchema::builder()
        .title("Approve tool call")
        .build()
        .expect("a schema without required fields builds")
}

/// Appends to the call log unless the project turned it off. A failed
/// insert is logged, not surfaced: the call itself already happened.
pub async fn log_call(store: &Store, settings: &ProjectSettings, entry: &CallEntry) {
    if settings.call_log_days == 0 {
        return;
    }
    if let Err(e) = store.append_call(entry).await {
        tracing::error!(error = %e, tool = %entry.tool_name, "could not write call log entry");
    }
}

/// Runs a stored tool (enabled or not) and shapes its response with the
/// project's settings. Shared by MCP `tools/call` and the UI test console so
/// both report identical results. Every run goes to the call log; SQL writes
/// also go to the audit log.
pub async fn run_tool(
    store: &Store,
    exec: &Executors,
    tool: &ToolDef,
    args: &Map<String, Value>,
    settings: &ProjectSettings,
    caller: &Caller,
) -> Result<ToolRun, StoreError> {
    let ts = chrono::Utc::now();
    let started = Instant::now();
    let run = execute(store, exec, tool, args, settings, caller.origin).await?;
    let entry = CallEntry {
        id: Uuid::new_v4(),
        project_id: tool.project_id,
        ts,
        tool_id: Some(tool.id),
        tool_name: tool.name.clone(),
        origin: caller.origin.to_owned(),
        client: caller.client.clone(),
        arguments: logged_arguments(args),
        status: if run.is_error {
            CallStatus::Error
        } else {
            CallStatus::Ok
        },
        duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        response_chars: run.text.chars().count() as u64,
        upstream_status: match &run.outcome {
            RunOutcome::Http(Ok(o)) => Some(o.status),
            _ => None,
        },
        error: run.is_error.then(|| logged_error(&run.text)),
        approval: caller.approval.clone(),
    };
    log_call(store, settings, &entry).await;
    Ok(run)
}

async fn execute(
    store: &Store,
    exec: &Executors,
    tool: &ToolDef,
    args: &Map<String, Value>,
    settings: &ProjectSettings,
    origin: &str,
) -> Result<ToolRun, StoreError> {
    let spec = match &tool.kind {
        ToolKind::Http(spec) => spec,
        ToolKind::Sql(spec) => {
            let conn = match store
                .get_connection(tool.project_id, spec.connection_id)
                .await
            {
                Ok(c) => c,
                Err(StoreError::NotFound(_)) => {
                    let outcome = Err(SqlError::Config(format!(
                        "tool `{}` references a missing connection",
                        tool.name
                    )));
                    return Ok(ToolRun::from_sql(outcome, tool, settings));
                }
                Err(e) => return Err(e),
            };
            let run = exec.sql.run(&conn, spec, args).await;
            if let Some(a) = run.audit {
                let entry = AuditEntry {
                    id: Uuid::new_v4(),
                    project_id: tool.project_id,
                    ts: chrono::Utc::now(),
                    tool_id: Some(tool.id),
                    tool_name: tool.name.clone(),
                    connection_id: Some(conn.id),
                    connection_name: conn.name.clone(),
                    origin: origin.to_owned(),
                    statement_class: a.class.as_str().to_owned(),
                    sql: a.sql,
                    params: a.params,
                    rows_affected: a.rows_affected,
                    outcome: a.outcome,
                    error: a.error,
                };
                // The write already happened; a failed audit insert is logged,
                // not turned into a tool error.
                if let Err(e) = store.append_audit(&entry).await {
                    tracing::error!(error = %e, tool = %tool.name, "could not write audit entry");
                }
            }
            return Ok(ToolRun::from_sql(run.result, tool, settings));
        }
    };
    let auth = match spec.auth_ref {
        Some(id) => match store.get_auth_config(tool.project_id, id).await {
            Ok(a) => Some(a),
            Err(StoreError::NotFound(_)) => {
                let outcome = Err(ExecError::Config(format!(
                    "tool `{}` references a missing auth config",
                    tool.name
                )));
                return Ok(ToolRun::from_outcome(outcome, tool, settings));
            }
            Err(e) => return Err(e),
        },
        None => None,
    };
    let outcome = match auth.as_ref() {
        Some(a) if a.kind.is_oauth() => oauth::execute(store, &exec.http, spec, a, args).await,
        _ => exec.http.execute(spec, auth.as_ref(), args).await,
    };
    Ok(ToolRun::from_outcome(outcome, tool, settings))
}

/// jq, null stripping and the size cap, as configured for the project.
/// Returns the agent text and whether shaping changed it.
fn shaped_text(
    body: &str,
    content_type: Option<&str>,
    tool: &ToolDef,
    settings: &ProjectSettings,
) -> (String, bool) {
    let opts = ShapeOptions {
        strip_nulls: settings.strip_nulls,
        max_chars: settings.max_response_chars as usize,
    };
    let s = shape_body(body, content_type, tool.response_shape.as_deref(), &opts);
    let changed = s.transformed || s.truncated || s.shape_error.is_some();
    (s.text, changed)
}

fn invalid_args_text(errs: &forge_core::ValidationErrors) -> String {
    let lines: Vec<String> = errs
        .errors
        .iter()
        .map(|e| format!("- `{}`: {}", e.field, e.message))
        .collect();
    format!("Invalid arguments:\n{}", lines.join("\n"))
}

impl ToolRun {
    fn from_sql(
        outcome: Result<SqlOutcome, SqlError>,
        tool: &ToolDef,
        settings: &ProjectSettings,
    ) -> Self {
        let mut shaped = None;
        let (is_error, text) = match &outcome {
            Ok(out) => {
                let body = out.result_json().to_string();
                let (text, changed) = shaped_text(&body, Some("application/json"), tool, settings);
                if changed {
                    shaped = Some(text.clone());
                }
                (false, text)
            }
            Err(SqlError::InvalidArgs(errs)) => (true, invalid_args_text(errs)),
            Err(SqlError::Rejected(m)) => (true, format!("Query rejected: {m}")),
            Err(SqlError::Config(m)) => (true, format!("Configuration error: {m}")),
            Err(SqlError::Connect(m)) => (true, format!("Database connection failed: {m}")),
            Err(SqlError::Timeout(ms)) => (
                true,
                format!(
                    "The query did not finish within {ms} ms. Try narrower arguments; \
                     an admin can raise the connection's timeout."
                ),
            ),
            Err(SqlError::Database(m)) => (true, format!("Database error: {m}")),
            Err(e @ SqlError::TooManyRows { .. }) => (
                true,
                format!("Not done: {e}. Narrow the WHERE clause, or check the count first with dry_run: true."),
            ),
        };
        Self {
            outcome: RunOutcome::Sql(outcome),
            is_error,
            text,
            shaped,
        }
    }

    fn from_outcome(
        outcome: Result<HttpOutcome, ExecError>,
        tool: &ToolDef,
        settings: &ProjectSettings,
    ) -> Self {
        let mut shaped = None;
        let (is_error, text) = match &outcome {
            Ok(out) if out.is_success() && !out.body.is_empty() => {
                let (mut text, changed) =
                    shaped_text(&out.body, out.content_type.as_deref(), tool, settings);
                if out.truncated {
                    text.push_str(
                        "\n\n[The upstream body exceeded the download limit and was cut before shaping.]",
                    );
                }
                if changed {
                    shaped = Some(text.clone());
                }
                (false, text)
            }
            Ok(out) => outcome_text(out),
            Err(ExecError::InvalidArgs(errs)) => (true, invalid_args_text(errs)),
            Err(ExecError::Config(m)) => (true, format!("Configuration error: {m}")),
            Err(ExecError::Request(m)) => (true, format!("Request failed: {m}")),
        };
        Self {
            outcome: RunOutcome::Http(outcome),
            is_error,
            text,
            shaped,
        }
    }
}

fn internal(e: StoreError) -> ErrorData {
    tracing::error!(error = %e, "store error during MCP request");
    ErrorData::internal_error("internal error", None)
}

/// Text for empty successes and upstream errors (shaping doesn't apply).
fn outcome_text(out: &HttpOutcome) -> (bool, String) {
    if out.is_success() {
        return (false, format!("HTTP {} (empty body)", out.status_line()));
    }
    let mut body: String = out.body.chars().take(MAX_ERROR_BODY_CHARS).collect();
    if body.len() < out.body.len() || out.truncated {
        body.push_str("\n[truncated]");
    }
    let detail = if body.is_empty() {
        "(empty body)".to_owned()
    } else {
        format!("Response body:\n{body}")
    };
    (
        true,
        format!(
            "Upstream returned HTTP {} for {} {}.\n{detail}",
            out.status_line(),
            out.method,
            out.url
        ),
    )
}

fn connection_of<'a>(
    t: &ToolDef,
    conns: &'a HashMap<Uuid, DbConnection>,
) -> Option<&'a DbConnection> {
    t.kind.connection_ref().and_then(|id| conns.get(&id))
}

/// What an agent is told about a SQL tool. Exploration tools always get a
/// description generated from the connection (driver, limits, allowlist,
/// masks), after the user's own text if any; curated queries use the user's
/// text, or a generated one when it's empty.
pub fn sql_tool_description(t: &ToolDef, spec: &SqlSpec, conn: Option<&DbConnection>) -> String {
    let user = t.description.trim();
    let db = conn.map_or_else(
        || "the configured database".to_owned(),
        |c| format!("the `{}` database ({})", c.name, driver_label(c)),
    );
    let writes = conn.is_some_and(|c| forge_exec_sql::tool_writes(spec, c));
    let write_note = conn.map_or_else(String::new, |c| {
        format!(
            "Changes data. Returns {{affected_rows, committed, dry_run}} (and any RETURNING rows). At most {} rows may \
             change per call; a larger change is undone. Pass dry_run: true to see the count without changing anything.",
            c.max_affected_rows
        )
    });
    let generated = match &spec.mode {
        SqlMode::Query { .. } if writes && !user.is_empty() => return format!("{user}\n\n{write_note}"),
        SqlMode::Query { .. } if !user.is_empty() => return user.to_owned(),
        SqlMode::Query { .. } if writes => format!("Runs a fixed statement on {db}. {write_note}"),
        SqlMode::Query { .. } => {
            format!("Runs a fixed read-only query on {db}. Returns {{columns, rows, truncated}}.")
        }
        SqlMode::ListTables => format!(
            "Lists the tables and views you can query in {db}. Returns {{columns: [schema, name, type], rows}}. \
             Then use describe_table to see a table's columns."
        ),
        SqlMode::DescribeTable => format!(
            "Shows the columns of one table in {db}: type, nullable, default, primary key, and whether its \
             values are masked."
        ),
        SqlMode::RunSql if writes => {
            let ddl = conn.is_some_and(|c| c.allow_ddl);
            let mut d = format!(
                "Runs one SQL statement on {db}: SELECT, INSERT, UPDATE or DELETE{}. UPDATE and DELETE need a WHERE \
                 clause; multiple statements, admin statements and system catalogs are refused. SELECT returns \
                 {{columns, rows, truncated}}. {write_note}",
                if ddl { ", or a schema change (CREATE, ALTER, DROP)" } else { "" }
            );
            if let Some(c) = conn {
                if c.table_allowlist.is_empty() {
                    d.push_str(" All non-system tables are available.");
                } else {
                    d.push_str(&format!(" Tables you may use: {}.", c.table_allowlist.join(", ")));
                }
                if !c.column_masks.is_empty() {
                    let masks: Vec<String> = c.column_masks.iter().map(|m| m.to_string()).collect();
                    d.push_str(&format!(
                        " Masked columns ({}) can't be used in statements.",
                        masks.join(", ")
                    ));
                }
            }
            d
        }
        SqlMode::RunSql => {
            let mut d = format!(
                "Runs one read-only SQL query on {db}. Only SELECT (or WITH … SELECT) is allowed; writes, \
                 multiple statements and system catalogs are refused. Returns {{columns, rows, truncated}}."
            );
            if let Some(c) = conn {
                d.push_str(&format!(
                    " At most {} rows; queries stop after {} s.",
                    c.row_limit,
                    f64::from(c.timeout_ms) / 1000.0
                ));
                if c.table_allowlist.is_empty() {
                    d.push_str(" All non-system tables are available.");
                } else {
                    d.push_str(&format!(" Tables you may read: {}.", c.table_allowlist.join(", ")));
                }
                if !c.column_masks.is_empty() {
                    let masks: Vec<String> = c.column_masks.iter().map(|m| m.to_string()).collect();
                    d.push_str(&format!(
                        " Masked columns ({}) can't be selected, filtered on or used in expressions; \
                         `SELECT *` shows them as [masked].",
                        masks.join(", ")
                    ));
                }
            }
            d
        }
    };
    if user.is_empty() {
        generated
    } else {
        format!("{user}\n\n{generated}")
    }
}

fn driver_label(c: &DbConnection) -> &'static str {
    match c.driver {
        forge_core::Driver::Postgres => "PostgreSQL",
        forge_core::Driver::Mysql => "MySQL",
        forge_core::Driver::Sqlite => "SQLite",
    }
}

/// MCP view of a stored tool. Input schema is derived, never stored. SQL
/// tools need their connection to describe themselves fully.
pub fn to_mcp_tool(t: &ToolDef, conn: Option<&DbConnection>) -> Tool {
    let spec = match &t.kind {
        ToolKind::Http(spec) => spec,
        ToolKind::Sql(spec) => {
            let description = sql_tool_description(t, spec, conn);
            let writes = conn.is_some_and(|c| forge_exec_sql::tool_writes(spec, c));
            let schema = match conn {
                Some(c) => forge_exec_sql::tool_input(spec, c).schema,
                None => spec.derive_input().schema,
            };
            // Only the configured database is touched; writers are destructive.
            let annotations = ToolAnnotations::new()
                .read_only(!writes)
                .destructive(writes)
                .open_world(false);
            return Tool::new(t.name.clone(), description, schema).with_annotations(annotations);
        }
    };
    let description = if t.description.trim().is_empty() {
        format!("{} {}", method_str(spec.method), spec.path_template)
    } else {
        t.description.clone()
    };
    let read_only = matches!(
        spec.method,
        HttpMethod::Get | HttpMethod::Head | HttpMethod::Options
    );
    let mut annotations = ToolAnnotations::new().read_only(read_only).open_world(true);
    if spec.method == HttpMethod::Delete {
        annotations = annotations.destructive(true);
    }
    Tool::new(t.name.clone(), description, spec.derive_input().schema).with_annotations(annotations)
}

fn method_str(m: HttpMethod) -> &'static str {
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

impl ServerHandler for ForgeMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_tool_list_changed()
                .build(),
        )
        .with_server_info(Implementation::new("mcp-forge", env!("CARGO_PKG_VERSION")))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let settings = self.settings().await?;
        if settings.tool_search {
            return Ok(ListToolsResult::with_all_items(search_tool_defs()));
        }
        let tools = self.exposed_tools(&settings).await?;
        let conns = self.connections().await?;
        Ok(ListToolsResult::with_all_items(
            tools
                .iter()
                .map(|t| to_mcp_tool(t, connection_of(t, &conns)))
                .collect(),
        ))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        // Stateless (2026-07-28) clients send their identity per request;
        // initialize-based sessions sent it once.
        let client = context
            .meta
            .client_info()
            .or_else(|| context.peer.peer_info().map(|i| i.client_info.clone()))
            .map(|c| format!("{} {}", c.name, c.version));
        let caller = Caller::mcp(client);
        let (peer, ct) = (&context.peer, &context.ct);
        let args = request.arguments.unwrap_or_default();
        let settings = self.settings().await?;
        let name: &str = request.name.as_ref();
        let res = if settings.tool_search && name == SEARCH_TOOL {
            self.search(&args, &settings).await
        } else if settings.tool_search && name == CALL_TOOL {
            match args.get("name").and_then(Value::as_str) {
                None => Ok(result(
                    true,
                    "Invalid arguments:\n- `name`: required argument is missing".into(),
                )),
                Some(SEARCH_TOOL | CALL_TOOL) => {
                    Ok(result(true, "call_tool can't call the meta-tools".into()))
                }
                Some(inner) => match args.get("arguments") {
                    None | Some(Value::Null) => {
                        self.call_named(inner, &Map::new(), &settings, &caller, peer, ct)
                            .await
                    }
                    Some(Value::Object(m)) => {
                        self.call_named(inner, m, &settings, &caller, peer, ct)
                            .await
                    }
                    Some(_) => Ok(result(
                        true,
                        "Invalid arguments:\n- `arguments`: expected object".into(),
                    )),
                },
            }
        } else {
            self.call_named(name, &args, &settings, &caller, peer, ct)
                .await
        };
        res.map(Into::into)
    }

    async fn on_initialized(&self, context: NotificationContext<RoleServer>) {
        let peer = context.peer.clone();
        let mut rx = self.changes.subscribe();
        let project_id = self.project_id;
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(30));
            loop {
                tokio::select! {
                    msg = rx.recv() => match msg {
                        Ok(id) if id != project_id => {}
                        Err(broadcast::error::RecvError::Closed) => break,
                        _ => {
                            if peer.notify_tool_list_changed().await.is_err() {
                                break;
                            }
                        }
                    },
                    _ = tick.tick() => {
                        if peer.is_transport_closed() {
                            break;
                        }
                    }
                }
            }
        });
    }

    fn accepted_subscription_filter(
        &self,
        _requested: &SubscriptionFilter,
    ) -> Option<SubscriptionFilter> {
        Some(SubscriptionFilter::builder().tools_list_changed().build())
    }

    async fn listen(&self, context: SubscriptionContext) -> Result<(), ErrorData> {
        let mut rx = self.changes.subscribe();
        loop {
            tokio::select! {
                () = context.cancelled() => return Ok(()),
                msg = rx.recv() => match msg {
                    Ok(id) if id != self.project_id => {}
                    Err(broadcast::error::RecvError::Closed) => {
                        context.cancelled().await;
                        return Ok(());
                    }
                    _ => {
                        if context.sink().notify_tool_list_changed().await.is_err() {
                            return Ok(());
                        }
                    }
                },
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StdioError {
    #[error("project `{0}` not found")]
    UnknownProject(String),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("MCP stdio transport failed: {0}")]
    Transport(String),
}

/// Serves `slug`'s tools over stdin/stdout until the client disconnects.
///
/// Edits made by another process (e.g. `mcp-forge serve`) are detected by
/// polling the store every `poll` and announced as list changes.
pub async fn serve_stdio(
    store: Store,
    mut exec: Executors,
    slug: &str,
    poll: Duration,
) -> Result<(), StdioError> {
    let project = match store.get_project_by_slug(slug).await {
        Ok(p) => p,
        Err(StoreError::NotFound(_)) => return Err(StdioError::UnknownProject(slug.into())),
        Err(e) => return Err(e.into()),
    };
    let changes = ChangeBus::new();
    tokio::spawn(watch_store(
        store.clone(),
        project.id,
        changes.clone(),
        poll,
    ));
    // No web UI in this process: approvals can only come from the client.
    exec.approvals = approvals::Approvals::without_ui();
    let handler = ForgeMcp::new(store, project.id, exec, changes);
    tracing::info!(project = slug, "serving MCP over stdio");
    let running = handler
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|e| StdioError::Transport(e.to_string()))?;
    running
        .waiting()
        .await
        .map_err(|e| StdioError::Transport(e.to_string()))?;
    Ok(())
}

/// Publishes a change whenever the project's tool fingerprint changes.
pub async fn watch_store(store: Store, project_id: Uuid, changes: ChangeBus, every: Duration) {
    let mut last = store.tools_fingerprint(project_id).await.ok();
    let mut tick = tokio::time::interval(every);
    loop {
        tick.tick().await;
        match store.tools_fingerprint(project_id).await {
            Ok(fp) if last.as_ref() != Some(&fp) => {
                last = Some(fp);
                changes.notify(project_id);
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "tool change poll failed"),
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/lib.rs"]
mod tests;
