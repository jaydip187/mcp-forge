use std::sync::atomic::{AtomicUsize, Ordering};

use forge_core::SecretRef;
use serde_json::json;
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::MapSecretResolver;

fn cfg(kind: Value) -> AuthConfig {
    AuthConfig {
        id: Uuid::new_v4(),
        project_id: Uuid::new_v4(),
        name: "api".into(),
        kind: serde_json::from_value(kind).unwrap(),
        secret_ref: SecretRef::parse("env:CLIENT_SECRET").unwrap(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn executor() -> HttpExecutor {
    HttpExecutor::new(Arc::new(
        MapSecretResolver::default().with("env:CLIENT_SECRET", "s3cret"),
    ))
}

#[test]
fn pkce_matches_rfc_7636() {
    assert_eq!(
        pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    );
    let (v, c) = pkce_pair();
    assert_eq!(pkce_challenge(&v), c);
    assert!(v.len() >= 43 && !v.contains(['+', '/', '=']));
    assert_ne!(random_token(16), random_token(16));
}

#[test]
fn authorize_url_carries_the_flow() {
    let c = cfg(json!({
        "type": "oauth2_authorization_code",
        "auth_url": "https://accounts.example.com/o/auth?prompt=consent",
        "token_url": "https://oauth.example.com/token",
        "client_id": "my client",
        "scopes": ["email", "profile"]
    }));
    let url = authorize_url(&c, "http://127.0.0.1:7410/oauth/callback", "st", "ch").unwrap();
    let q: HashMap<String, String> = url.query_pairs().into_owned().collect();
    assert_eq!(q["prompt"], "consent", "existing query kept");
    assert_eq!(q["response_type"], "code");
    assert_eq!(q["client_id"], "my client");
    assert_eq!(q["redirect_uri"], "http://127.0.0.1:7410/oauth/callback");
    assert_eq!(q["state"], "st");
    assert_eq!(q["code_challenge"], "ch");
    assert_eq!(q["code_challenge_method"], "S256");
    assert_eq!(q["scope"], "email profile");

    let cc = cfg(json!({
        "type": "oauth2_client_credentials",
        "token_url": "https://oauth.example.com/token",
        "client_id": "x"
    }));
    assert!(authorize_url(&cc, "r", "s", "c").is_err());
}

#[test]
fn expiry_margin() {
    let now = Utc::now();
    assert!(usable(None, now));
    assert!(usable(Some(now + Duration::minutes(5)), now));
    assert!(!usable(Some(now + Duration::seconds(30)), now));
}

#[tokio::test]
async fn client_credentials_in_the_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(header("content-type", "application/x-www-form-urlencoded"))
        .and(body_string_contains("grant_type=client_credentials"))
        .and(body_string_contains("scope=read+write"))
        .and(body_string_contains(
            "audience=https%3A%2F%2Fapi.example.com",
        ))
        .and(body_string_contains("client_id=cid"))
        .and(body_string_contains("client_secret=s3cret"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "tok-1", "token_type": "Bearer", "expires_in": 120
        })))
        .expect(1)
        .mount(&server)
        .await;
    let c = cfg(json!({
        "type": "oauth2_client_credentials",
        "token_url": format!("{}/token", server.uri()),
        "client_id": "cid",
        "scopes": ["read", "write"],
        "audience": "https://api.example.com"
    }));
    let t = executor().client_credentials_token(&c).await.unwrap();
    assert_eq!(t.access_token, "tok-1");
    assert!(t.expires_at.unwrap() > Utc::now() + Duration::seconds(100));
}

#[tokio::test]
async fn basic_client_auth_and_redacted_errors() {
    let server = MockServer::start().await;
    // base64("cid:s3cret")
    Mock::given(method("POST"))
        .and(header("authorization", "Basic Y2lkOnMzY3JldA=="))
        .and(body_string_contains("grant_type=refresh_token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": "invalid_grant",
            "error_description": "refresh token rt-123 expired, client secret s3cret"
        })))
        .mount(&server)
        .await;
    let c = cfg(json!({
        "type": "oauth2_authorization_code",
        "auth_url": "https://accounts.example.com/auth",
        "token_url": format!("{}/token", server.uri()),
        "client_id": "cid",
        "client_auth": "basic"
    }));
    let err = executor()
        .refresh_oauth(&c, "rt-123")
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("answered 400: invalid_grant"), "{err}");
    assert!(!err.contains("s3cret") && !err.contains("rt-123"), "{err}");
}

#[tokio::test]
async fn non_bearer_tokens_are_refused() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "t", "token_type": "mac"
        })))
        .mount(&server)
        .await;
    let c = cfg(json!({
        "type": "oauth2_client_credentials",
        "token_url": format!("{}/token", server.uri()),
        "client_id": "cid"
    }));
    let err = executor().client_credentials_token(&c).await.unwrap_err();
    assert!(err.to_string().contains("only bearer"));
}

#[tokio::test]
async fn cache_fetches_once_and_refetches_a_rejected_token() {
    let cache = TokenCache::default();
    let c = cfg(json!({
        "type": "oauth2_client_credentials",
        "token_url": "https://oauth.example.com/token",
        "client_id": "cid"
    }));
    let calls = AtomicUsize::new(0);
    let fetch = || async {
        let n = calls.fetch_add(1, Ordering::SeqCst) + 1;
        Ok((format!("tok-{n}"), None))
    };
    assert_eq!(cache.get_or_fetch(&c, None, fetch).await.unwrap(), "tok-1");
    assert_eq!(cache.get_or_fetch(&c, None, fetch).await.unwrap(), "tok-1");
    // Someone else already replaced the rejected token: keep the new one.
    assert_eq!(
        cache.get_or_fetch(&c, Some("tok-0"), fetch).await.unwrap(),
        "tok-1"
    );
    assert_eq!(
        cache.get_or_fetch(&c, Some("tok-1"), fetch).await.unwrap(),
        "tok-2"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    // Editing the config starts over.
    let edited = AuthConfig {
        updated_at: c.updated_at + Duration::seconds(1),
        ..c.clone()
    };
    assert_eq!(
        cache.get_or_fetch(&edited, None, fetch).await.unwrap(),
        "tok-3"
    );
    cache.forget(c.id);
    assert_eq!(cache.get_or_fetch(&c, None, fetch).await.unwrap(), "tok-4");
}
