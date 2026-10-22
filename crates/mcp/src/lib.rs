//! `rmcp` server handler backed by the store: one handler serves one project.
//!
//! Tool definitions are read from the store on every request, so edits are
//! visible immediately. Tool-list changes are announced to connected clients
//! through a [`ChangeBus`]: as `notifications/tools/list_changed` for
//! initialize-based sessions and through `subscriptions/listen` for
//! 2026-07-28 clients.

use std::sync::Arc;
use std::time::Duration;

use std::collections::HashMap;

use forge_core::{
    DbConnection, HttpMethod, ProjectSettings, SecretResolver, SqlMode, SqlSpec, ToolDef, ToolKind,
};
use forge_exec_http::shape::{shape_body, ShapeOptions};
use forge_exec_http::{ExecError, HttpExecutor, HttpOutcome};
use forge_exec_sql::{SqlError, SqlExecutor, SqlOutcome};
use forge_store::{Store, StoreError};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, SubscriptionFilter,
    Tool, ToolAnnotations,
};
use rmcp::service::{NotificationContext, RequestContext, SubscriptionContext};
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde_json::{json, Map, Value};
use tokio::sync::broadcast;
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
}

impl Executors {
    pub fn new(secrets: Arc<dyn SecretResolver>) -> Self {
        Self {
            http: HttpExecutor::new(secrets.clone()),
            sql: SqlExecutor::new(secrets),
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
        Ok(tools.into_iter().filter(|t| settings.exposes(t)).collect())
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
    ) -> Result<CallToolResult, ErrorData> {
        let tool = match self.store.get_tool_by_name(self.project_id, name).await {
            Ok(t) if settings.exposes(&t) => t,
            Ok(_) | Err(StoreError::NotFound(_)) => {
                return Err(ErrorData::invalid_params(
                    format!("unknown tool `{name}`"),
                    None,
                ));
            }
            Err(e) => return Err(internal(e)),
        };
        let run = run_tool(&self.store, &self.exec, &tool, args, settings)
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

/// Runs a stored tool (enabled or not) and shapes its response with the
/// project's settings. Shared by MCP `tools/call` and the UI test console so
/// both report identical results.
pub async fn run_tool(
    store: &Store,
    exec: &Executors,
    tool: &ToolDef,
    args: &Map<String, Value>,
    settings: &ProjectSettings,
) -> Result<ToolRun, StoreError> {
    let spec = match &tool.kind {
        ToolKind::Http(spec) => spec,
        ToolKind::Sql(spec) => {
            let outcome = match store
                .get_connection(tool.project_id, spec.connection_id)
                .await
            {
                Ok(conn) => exec.sql.run_query(&conn, spec, args).await,
                Err(StoreError::NotFound(_)) => Err(SqlError::Config(format!(
                    "tool `{}` references a missing connection",
                    tool.name
                ))),
                Err(e) => return Err(e),
            };
            return Ok(ToolRun::from_sql(outcome, tool, settings));
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
    let outcome = exec.http.execute(spec, auth.as_ref(), args).await;
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
    let generated = match &spec.mode {
        SqlMode::Query { .. } if !user.is_empty() => return user.to_owned(),
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
            // Curated queries only read, and only from the configured database.
            let annotations = ToolAnnotations::new()
                .read_only(true)
                .destructive(false)
                .open_world(false);
            return Tool::new(t.name.clone(), description, spec.derive_input().schema)
                .with_annotations(annotations);
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
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
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
                        self.call_named(inner, &Map::new(), &settings).await
                    }
                    Some(Value::Object(m)) => self.call_named(inner, m, &settings).await,
                    Some(_) => Ok(result(
                        true,
                        "Invalid arguments:\n- `arguments`: expected object".into(),
                    )),
                },
            }
        } else {
            self.call_named(name, &args, &settings).await
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
    exec: Executors,
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
mod tests {
    use super::*;

    fn tool(name: &str, desc: &str, path: &str, tags: &[&str]) -> ToolDef {
        serde_json::from_value(json!({
            "id": uuid::Uuid::new_v4(), "project_id": uuid::Uuid::new_v4(),
            "name": name, "description": desc, "enabled": true, "tags": tags,
            "kind": { "type": "http", "method": "GET", "base_url": "https://x.io", "path_template": path },
            "source": { "type": "manual" }, "overrides": {}, "response_shape": null,
            "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z"
        }))
        .unwrap()
    }

    #[test]
    fn search_ranks_name_over_tags_over_description() {
        let tools = [
            tool(
                "list_orders",
                "Lists orders for a user",
                "/orders",
                &["orders"],
            ),
            tool("get_user", "Fetch one user", "/users/{id}", &["users"]),
            tool("ping", "Health check", "/ping", &[]),
        ];
        let names = |q: &str, n| {
            search_tools(&tools, q, n)
                .iter()
                .map(|t| t.name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(names("user", 10), ["get_user", "list_orders"]);
        assert_eq!(names("orders", 10), ["list_orders"]);
        assert_eq!(names("health", 10), ["ping"]);
        assert!(names("nothing-matches", 10).is_empty());
        assert_eq!(names("", 2).len(), 2, "empty query lists tools");
        assert_eq!(names("user", 1), ["get_user"]);
    }
}
