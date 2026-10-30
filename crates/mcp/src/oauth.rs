//! Runs HTTP tools whose auth config uses OAuth2: gets an access token
//! (cached; refreshed from the stored refresh token for the authorization
//! code flow) and retries once with a fresh token when the upstream says 401.

use chrono::Utc;
use forge_core::{AuthConfig, AuthKind, HttpSpec};
use forge_exec_http::oauth::usable;
use forge_exec_http::{ExecError, HttpExecutor, HttpOutcome};
use forge_store::{OAuthToken, Store};
use serde_json::{Map, Value};

fn store_error(e: forge_store::StoreError) -> ExecError {
    tracing::error!(error = %e, "store error while getting an OAuth token");
    ExecError::Config("internal error reading OAuth tokens; see server logs".into())
}

/// An access token for `cfg`. With `stale` (a token the upstream rejected),
/// a fresh one is fetched unless someone already did.
pub async fn access_token(
    store: &Store,
    http: &HttpExecutor,
    cfg: &AuthConfig,
    stale: Option<&str>,
) -> Result<String, ExecError> {
    match &cfg.kind {
        AuthKind::OAuth2ClientCredentials { .. } => {
            http.tokens()
                .get_or_fetch(cfg, stale, || async {
                    let t = http.client_credentials_token(cfg).await?;
                    Ok((t.access_token, t.expires_at))
                })
                .await
        }
        AuthKind::OAuth2AuthorizationCode { .. } => {
            http.tokens()
                .get_or_fetch(cfg, stale, || async {
                    stored_or_refreshed(store, http, cfg, stale).await
                })
                .await
        }
        _ => Err(ExecError::Config(format!(
            "auth config `{}` isn't an OAuth2 config",
            cfg.name
        ))),
    }
}

async fn stored_or_refreshed(
    store: &Store,
    http: &HttpExecutor,
    cfg: &AuthConfig,
    stale: Option<&str>,
) -> Result<(String, Option<chrono::DateTime<Utc>>), ExecError> {
    let reconnect = |why: &str| {
        ExecError::Config(format!(
            "auth config `{}` {why}: open the project's Auth tab in mcp-forge and click Connect",
            cfg.name
        ))
    };
    let stored = store
        .get_oauth_token(cfg.id)
        .await
        .map_err(store_error)?
        .ok_or_else(|| reconnect("isn't connected yet"))?;
    if usable(stored.expires_at, Utc::now()) && Some(stored.access_token.as_str()) != stale {
        return Ok((stored.access_token, stored.expires_at));
    }
    let Some(refresh) = stored.refresh_token.clone() else {
        return Err(reconnect("has an expired token and no refresh token"));
    };
    let fresh = http
        .refresh_oauth(cfg, &refresh)
        .await
        .map_err(|e| match e {
            // The provider refused the refresh token: only a new sign-in helps.
            ExecError::Config(m) => ExecError::Config(format!(
                "{m}. Reconnect it in mcp-forge (project → Auth → Connect)"
            )),
            other => other,
        })?;
    let token = OAuthToken {
        access_token: fresh.access_token,
        // Providers that don't rotate refresh tokens keep the old one valid.
        refresh_token: fresh.refresh_token.or(Some(refresh)),
        expires_at: fresh.expires_at,
        scope: fresh.scope.or(stored.scope),
        connected_at: stored.connected_at,
        connected_by: stored.connected_by,
    };
    store
        .put_oauth_token(cfg.id, &token)
        .await
        .map_err(store_error)?;
    Ok((token.access_token, token.expires_at))
}

/// Executes with an OAuth token; a 401 gets one retry after fetching a fresh token.
pub async fn execute(
    store: &Store,
    http: &HttpExecutor,
    spec: &HttpSpec,
    cfg: &AuthConfig,
    args: &Map<String, Value>,
) -> Result<HttpOutcome, ExecError> {
    let token = access_token(store, http, cfg, None).await?;
    let out = http.execute_with_token(spec, cfg, &token, args).await?;
    if out.status != 401 {
        return Ok(out);
    }
    // Retry even if the provider handed back the same string: it may have
    // re-activated it. One retry at most.
    let fresh = access_token(store, http, cfg, Some(&token)).await?;
    tracing::info!(auth = %cfg.name, "upstream said 401; retrying with a fresh OAuth token");
    http.execute_with_token(spec, cfg, &fresh, args).await
}
