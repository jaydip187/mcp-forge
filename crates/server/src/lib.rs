//! axum application: REST API, UI and MCP endpoints.

pub mod api;
mod assets;
pub mod bundle;
pub mod config;
mod import;
pub mod login;
mod mcp_http;
mod reimport;
pub mod security;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{any, get};
use axum::{Json, Router};
use forge_core::SecretResolver;
use forge_mcp::{ChangeBus, Executors};
use forge_store::Store;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tower_http::trace::TraceLayer;

pub use security::Security;

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub exec: Executors,
    /// Announces tool-list changes to live MCP sessions.
    pub changes: ChangeBus,
    /// Cancelled on shutdown so long-lived MCP streams close.
    pub shutdown: CancellationToken,
    pub security: Security,
    /// Shown in the UI's stdio config snippet.
    pub data_dir: Option<PathBuf>,
    /// Setup code and sign-in backoff for UI accounts.
    pub login: Arc<login::Login>,
    mcp: Arc<mcp_http::McpServices>,
}

impl AppState {
    pub fn new(store: Store, secrets: Arc<dyn SecretResolver>) -> Self {
        Self {
            store,
            exec: Executors::new(secrets),
            changes: ChangeBus::new(),
            shutdown: CancellationToken::new(),
            security: Security::default(),
            data_dir: None,
            login: Arc::default(),
            mcp: Arc::default(),
        }
    }

    pub fn with_security(mut self, security: Security) -> Self {
        self.security = security;
        self
    }

    pub fn with_data_dir(mut self, dir: PathBuf) -> Self {
        self.data_dir = Some(dir);
        self
    }
}

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .nest(
            "/api",
            api::router()
                .merge(import::router())
                .merge(reimport::router())
                .merge(login::router()),
        )
        .route("/mcp/{slug}", any(mcp_http::handle))
        .fallback(assets::serve)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            security::guard,
        ))
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
    let app = app(state).into_make_service_with_connect_info::<SocketAddr>();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown.await;
            token.cancel();
        })
        .await
}
