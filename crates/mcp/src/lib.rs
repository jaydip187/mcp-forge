//! `rmcp` server handler backed by the store: one handler serves one project.
//!
//! Tool definitions are read from the store on every request, so edits are
//! visible immediately. Tool-list changes are announced to connected clients
//! through a [`ChangeBus`]: as `notifications/tools/list_changed` for
//! initialize-based sessions and through `subscriptions/listen` for
//! 2026-07-28 clients.

use std::time::Duration;

use forge_core::{HttpMethod, ToolDef, ToolKind};
use forge_exec_http::{ExecError, HttpExecutor, HttpOutcome};
use forge_store::{Store, StoreError};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, SubscriptionFilter,
    Tool, ToolAnnotations,
};
use rmcp::service::{NotificationContext, RequestContext, SubscriptionContext};
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde_json::Map;
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

/// MCP handler for a single project.
#[derive(Clone)]
pub struct ForgeMcp {
    store: Store,
    project_id: Uuid,
    exec: HttpExecutor,
    changes: ChangeBus,
}

impl ForgeMcp {
    pub fn new(store: Store, project_id: Uuid, exec: HttpExecutor, changes: ChangeBus) -> Self {
        Self {
            store,
            project_id,
            exec,
            changes,
        }
    }

    async fn enabled_tools(&self) -> Result<Vec<ToolDef>, ErrorData> {
        let tools = self
            .store
            .list_tools(self.project_id)
            .await
            .map_err(internal)?;
        Ok(tools.into_iter().filter(|t| t.enabled).collect())
    }

    async fn call_named(
        &self,
        name: &str,
        args: &Map<String, serde_json::Value>,
    ) -> Result<CallToolResult, ErrorData> {
        let tool = match self.store.get_tool_by_name(self.project_id, name).await {
            Ok(t) if t.enabled => t,
            Ok(_) | Err(StoreError::NotFound(_)) => {
                return Err(ErrorData::invalid_params(
                    format!("unknown tool `{name}`"),
                    None,
                ));
            }
            Err(e) => return Err(internal(e)),
        };
        let run = run_tool(&self.store, &self.exec, &tool, args)
            .await
            .map_err(internal)?;
        tracing::info!(
            tool = name,
            ok = !run.is_error,
            status = run.outcome.as_ref().ok().map(|o| o.status),
            "tool call"
        );
        let content = vec![ContentBlock::text(run.text)];
        Ok(if run.is_error {
            CallToolResult::error(content)
        } else {
            CallToolResult::success(content)
        })
    }
}

/// Result of running a tool: the raw outcome plus exactly what the agent sees.
#[derive(Debug)]
pub struct ToolRun {
    pub outcome: Result<HttpOutcome, ExecError>,
    pub is_error: bool,
    pub text: String,
}

/// Runs a stored tool (enabled or not). Shared by MCP `tools/call` and the
/// UI test console so both report identical results.
pub async fn run_tool(
    store: &Store,
    exec: &HttpExecutor,
    tool: &ToolDef,
    args: &Map<String, serde_json::Value>,
) -> Result<ToolRun, StoreError> {
    let ToolKind::Http(spec) = &tool.kind;
    let auth = match spec.auth_ref {
        Some(id) => match store.get_auth_config(tool.project_id, id).await {
            Ok(a) => Some(a),
            Err(StoreError::NotFound(_)) => {
                let outcome = Err(ExecError::Config(format!(
                    "tool `{}` references a missing auth config",
                    tool.name
                )));
                return Ok(ToolRun::from_outcome(outcome));
            }
            Err(e) => return Err(e),
        },
        None => None,
    };
    let outcome = exec.execute(spec, auth.as_ref(), args).await;
    Ok(ToolRun::from_outcome(outcome))
}

impl ToolRun {
    fn from_outcome(outcome: Result<HttpOutcome, ExecError>) -> Self {
        let (is_error, text) = match &outcome {
            Ok(out) => outcome_text(out),
            Err(ExecError::InvalidArgs(errs)) => {
                let lines: Vec<String> = errs
                    .errors
                    .iter()
                    .map(|e| format!("- `{}`: {}", e.field, e.message))
                    .collect();
                (
                    true,
                    format!(
                        "Invalid arguments:
{}",
                        lines.join(
                            "
"
                        )
                    ),
                )
            }
            Err(ExecError::Config(m)) => (true, format!("Configuration error: {m}")),
            Err(ExecError::Request(m)) => (true, format!("Request failed: {m}")),
        };
        Self {
            outcome,
            is_error,
            text,
        }
    }
}

fn internal(e: StoreError) -> ErrorData {
    tracing::error!(error = %e, "store error during MCP request");
    ErrorData::internal_error("internal error", None)
}

fn outcome_text(out: &HttpOutcome) -> (bool, String) {
    if out.is_success() {
        let mut body = out.body.clone();
        if body.is_empty() {
            body = format!("HTTP {} (empty body)", out.status_line());
        }
        if out.truncated {
            body.push_str(
                "

[response truncated: upstream body exceeded the size limit]",
            );
        }
        return (false, body);
    }
    let mut body: String = out.body.chars().take(MAX_ERROR_BODY_CHARS).collect();
    let cut = body.len() < out.body.len() || out.truncated;
    if cut {
        body.push_str(
            "
[truncated]",
        );
    }
    let detail = if body.is_empty() {
        "(empty body)".to_owned()
    } else {
        format!(
            "Response body:
{body}"
        )
    };
    (
        true,
        format!(
            "Upstream returned HTTP {} for {} {}.
{detail}",
            out.status_line(),
            out.method,
            out.url
        ),
    )
}

/// MCP view of a stored tool. Input schema is derived, never stored.
pub fn to_mcp_tool(t: &ToolDef) -> Tool {
    let ToolKind::Http(spec) = &t.kind;
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
        let tools = self.enabled_tools().await?;
        Ok(ListToolsResult::with_all_items(
            tools.iter().map(to_mcp_tool).collect(),
        ))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let args = request.arguments.unwrap_or_default();
        self.call_named(&request.name, &args).await.map(Into::into)
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
    exec: HttpExecutor,
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
