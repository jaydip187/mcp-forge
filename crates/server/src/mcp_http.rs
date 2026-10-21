//! Streamable HTTP MCP endpoint at `/mcp/{project_slug}`.
//!
//! rmcp's service factory cannot see the request path, so we keep one
//! `StreamableHttpService` per project (keyed by id, so renaming a slug keeps
//! existing sessions) and dispatch to it after resolving the slug.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::extract::{Path, Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use forge_mcp::ForgeMcp;
use forge_store::StoreError;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use serde_json::json;
use uuid::Uuid;

use crate::AppState;

type Service = StreamableHttpService<ForgeMcp, LocalSessionManager>;

#[derive(Default)]
pub struct McpServices {
    by_project: Mutex<HashMap<Uuid, Service>>,
}

impl McpServices {
    fn get_or_create(&self, state: &AppState, project_id: Uuid) -> Service {
        let mut map = self.by_project.lock().expect("mcp service map poisoned");
        map.entry(project_id)
            .or_insert_with(|| {
                let (store, exec, changes) = (
                    state.store.clone(),
                    state.exec.clone(),
                    state.changes.clone(),
                );
                let mut config = StreamableHttpServerConfig::default()
                    .with_cancellation_token(state.shutdown.child_token());
                if !state.security.loopback_hosts_only {
                    // Non-loopback binds are protected by the admin token instead.
                    config = config.disable_allowed_hosts();
                }
                StreamableHttpService::new(
                    move || {
                        Ok(ForgeMcp::new(
                            store.clone(),
                            project_id,
                            exec.clone(),
                            changes.clone(),
                        ))
                    },
                    Arc::new(LocalSessionManager::default()),
                    config,
                )
            })
            .clone()
    }
}

pub async fn handle(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    request: Request,
) -> Response {
    let project = match state.store.get_project_by_slug(&slug).await {
        Ok(p) => p,
        Err(StoreError::NotFound(_)) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": { "code": "not_found", "message": format!("no project with slug `{slug}`") } })),
            )
                .into_response();
        }
        Err(e) => {
            tracing::error!(error = %e, "resolving MCP project");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let service = state.mcp.get_or_create(&state, project.id);
    service.handle(request).await.map(axum::body::Body::new)
}
