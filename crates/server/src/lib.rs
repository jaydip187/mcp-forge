//! axum application: REST API, UI and MCP endpoints.

pub mod api;
pub mod config;
mod mcp_http;

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{any, get};
use axum::{Json, Router};
use forge_core::SecretResolver;
use forge_exec_http::HttpExecutor;
use forge_mcp::ChangeBus;
use forge_store::Store;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tower_http::trace::TraceLayer;

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub exec: HttpExecutor,
    /// Announces tool-list changes to live MCP sessions.
    pub changes: ChangeBus,
    /// Cancelled on shutdown so long-lived MCP streams close.
    pub shutdown: CancellationToken,
    mcp: Arc<mcp_http::McpServices>,
}

impl AppState {
    pub fn new(store: Store, secrets: Arc<dyn SecretResolver>) -> Self {
        Self {
            store,
            exec: HttpExecutor::new(secrets),
            changes: ChangeBus::new(),
            shutdown: CancellationToken::new(),
            mcp: Arc::default(),
        }
    }
}

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .nest("/api", api::router())
        .route("/mcp/{slug}", any(mcp_http::handle))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn healthz(State(state): State<AppState>) -> (StatusCode, Json<Value>) {
    match state.store.ping().await {
        Ok(()) => (StatusCode::OK, Json(json!({ "status": "ok" }))),
        Err(e) => {
            tracing::error!(error = %e, "health check failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "status": "error", "error": "database unavailable" })),
            )
        }
    }
}

/// Serves the app on an already-bound listener until `shutdown` resolves.
pub async fn serve(
    listener: TcpListener,
    state: AppState,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    let token = state.shutdown.clone();
    axum::serve(listener, app(state))
        .with_graceful_shutdown(async move {
            shutdown.await;
            token.cancel();
        })
        .await
}
