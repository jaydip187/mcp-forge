//! Stage 19: OAuth2 for upstream APIs — the Connect flow (authorization code
//! + PKCE), refresh on 401, client credentials, status and disconnect.

use std::collections::HashMap;
use std::sync::Arc;

use forge_exec_http::oauth::pkce_challenge;
use forge_exec_http::MapSecretResolver;
use forge_store::Store;
use mcp_forge::{AppState, Security};
use reqwest::header::LOCATION;
use reqwest::{StatusCode, Url};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

/// Matches a token request whose `code_verifier` hashes to `challenge`.
struct VerifierMatches(String);

impl wiremock::Match for VerifierMatches {
    fn matches(&self, req: &Request) -> bool {
        let form: HashMap<String, String> = url::form_urlencoded::parse(&req.body)
            .into_owned()
            .collect();
        form.get("code_verifier")
            .is_some_and(|v| pkce_challenge(v) == self.0)
    }
}

struct Harness {
    base: String,
    http: reqwest::Client,
    pid: String,
    provider: MockServer,
    store: Store,
}

impl Harness {
    async fn start() -> Self {
        let store = Store::open_in_memory().await.unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let secrets = MapSecretResolver::default().with("env:CLIENT_SECRET", "client-s3cret");
        tokio::spawn(mcp_forge::serve(
            listener,
            AppState::new(store.clone(), Arc::new(secrets)).with_security(Security {
                require_login: false,
                ..Security::default()
            }),
            std::future::pending(),
        ));
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let p: Value = http
            .post(format!("{base}/api/projects"))
            .json(&json!({ "name": "Demo" }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        Self {
            base,
            http,
            pid: p["id"].as_str().unwrap().to_owned(),
            provider: MockServer::start().await,
            store,
        }
    }

    fn url(&self, rest: &str) -> String {
        format!("{}/api/projects/{}{rest}", self.base, self.pid)
    }

    async fn post(&self, rest: &str, body: Value) -> Value {
        let r = self
            .http
            .post(self.url(rest))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert!(r.status().is_success(), "{}", r.text().await.unwrap());
        r.json().await.unwrap_or(Value::Null)
    }

    async fn auth(&self, kind: Value) -> String {
        let a = self
            .post(
                "/auth-configs",
                json!({ "name": format!("a{}", rand_suffix()), "kind": kind, "secret_ref": "env:CLIENT_SECRET" }),
            )
            .await;
        a["id"].as_str().unwrap().to_owned()
    }

    async fn tool(&self, name: &str, auth_id: &str) -> String {
        let t = self
            .post(
                "/tools",
                json!({
                    "name": name,
                    "kind": {
                        "type": "http", "method": "GET",
                        "base_url": self.provider.uri(), "path_template": "/me",
                        "params": [], "auth_ref": auth_id
                    }
                }),
            )
            .await;
        t["id"].as_str().unwrap().to_owned()
    }

    async fn run(&self, tool_id: &str) -> Value {
        self.post(
            &format!("/tools/{tool_id}/test"),
            json!({ "arguments": {} }),
        )
        .await
    }
}

fn rand_suffix() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..8].to_owned()
}

#[tokio::test]
async fn connect_then_refresh_on_401() {
    let h = Harness::start().await;
    let auth_id = h
        .auth(json!({
            "type": "oauth2_authorization_code",
            "auth_url": format!("{}/authorize", h.provider.uri()),
            "token_url": format!("{}/token", h.provider.uri()),
            "client_id": "cid",
            "scopes": ["profile"]
        }))
        .await;
    let tool_id = h.tool("me", &auth_id).await;

    // Not connected yet: the agent is told what to do.
    let run = h.run(&tool_id).await;
    assert!(
        run["error"].to_string().contains("isn't connected yet"),
        "{run}"
    );

    // Start: the authorize URL carries state and an S256 challenge.
    let started = h
        .post(&format!("/auth-configs/{auth_id}/oauth/start"), json!({}))
        .await;
    let authorize = Url::parse(started["authorize_url"].as_str().unwrap()).unwrap();
    let q: HashMap<String, String> = authorize.query_pairs().into_owned().collect();
    assert_eq!(q["client_id"], "cid");
    assert_eq!(q["scope"], "profile");
    assert_eq!(q["code_challenge_method"], "S256");
    let redirect = format!("{}/oauth/callback", h.base);
    assert_eq!(q["redirect_uri"], redirect);
    assert_eq!(started["redirect_uri"], redirect.as_str());

    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=authorization_code"))
        .and(body_string_contains("code=the-code"))
        .and(body_string_contains("client_secret=client-s3cret"))
        .and(VerifierMatches(q["code_challenge"].clone()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "acc-1", "refresh_token": "ref-1",
            "token_type": "Bearer", "expires_in": 3600, "scope": "profile"
        })))
        .expect(1)
        .mount(&h.provider)
        .await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .and(body_string_contains("refresh_token=ref-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "acc-2", "token_type": "bearer", "expires_in": 3600
        })))
        .expect(1)
        .mount(&h.provider)
        .await;
    // The upstream API has already revoked acc-1.
    Mock::given(method("GET"))
        .and(path("/me"))
        .and(header("authorization", "Bearer acc-1"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&h.provider)
        .await;
    Mock::given(method("GET"))
        .and(path("/me"))
        .and(header("authorization", "Bearer acc-2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "me": "alice" })))
        .mount(&h.provider)
        .await;

    // The provider sends the browser back.
    let cb = format!(
        "{}/oauth/callback?state={}&code=the-code",
        h.base, q["state"]
    );
    let r = h.http.get(&cb).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        r.headers()[LOCATION],
        format!("/#/p/{}/auth", h.pid).as_str()
    );
    let again = h.http.get(&cb).send().await.unwrap();
    assert_eq!(
        again.status(),
        StatusCode::BAD_REQUEST,
        "state is single use"
    );

    let status: Value = h
        .http
        .get(h.url("/oauth"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status[0]["auth_config_id"], auth_id.as_str());
    assert_eq!(status[0]["can_refresh"], true);
    assert_eq!(status[0]["scope"], "profile");
    assert!(
        status[0].get("access_token").is_none(),
        "tokens never leave the server"
    );

    // 401 with acc-1 → refresh → retry with acc-2.
    let run = h.run(&tool_id).await;
    assert_eq!(run["response"]["status"], 200, "{run}");
    assert!(run["agent_view"].to_string().contains("alice"), "{run}");
    let stored = h
        .store
        .get_oauth_token(auth_id.parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.access_token, "acc-2");
    assert_eq!(
        stored.refresh_token.as_deref(),
        Some("ref-1"),
        "kept when not rotated"
    );

    // Disconnect.
    let r = h
        .http
        .delete(h.url(&format!("/auth-configs/{auth_id}/oauth")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    let status: Value = h
        .http
        .get(h.url("/oauth"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status, json!([]));
    assert!(h.run(&tool_id).await["error"]
        .to_string()
        .contains("isn't connected yet"));
}

#[tokio::test]
async fn provider_errors_are_shown() {
    let h = Harness::start().await;
    let auth_id = h
        .auth(json!({
            "type": "oauth2_authorization_code",
            "auth_url": format!("{}/authorize", h.provider.uri()),
            "token_url": format!("{}/token", h.provider.uri()),
            "client_id": "cid"
        }))
        .await;
    let started = h
        .post(&format!("/auth-configs/{auth_id}/oauth/start"), json!({}))
        .await;
    let authorize = Url::parse(started["authorize_url"].as_str().unwrap()).unwrap();
    let state = authorize
        .query_pairs()
        .find(|(k, _)| k == "state")
        .unwrap()
        .1
        .into_owned();
    let r = h
        .http
        .get(format!(
            "{}/oauth/callback?state={state}&error=access_denied&error_description=%3Cb%3Enope%3C%2Fb%3E",
            h.base
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    let page = r.text().await.unwrap();
    assert!(
        page.contains("access_denied") && page.contains("&lt;b&gt;nope"),
        "escaped: {page}"
    );

    let r = h
        .http
        .get(format!("{}/oauth/callback?state=forged&code=x", h.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);

    // Client credentials configs have no Connect step.
    let cc = h
        .auth(json!({
            "type": "oauth2_client_credentials",
            "token_url": format!("{}/token", h.provider.uri()),
            "client_id": "cid"
        }))
        .await;
    let r = h
        .http
        .post(h.url(&format!("/auth-configs/{cc}/oauth/start")))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn client_credentials_token_is_cached() {
    let h = Harness::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=client_credentials"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "cc-1", "token_type": "Bearer", "expires_in": 3600
        })))
        .expect(1)
        .mount(&h.provider)
        .await;
    Mock::given(method("GET"))
        .and(path("/me"))
        .and(header("authorization", "Bearer cc-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "me": "service" })))
        .mount(&h.provider)
        .await;
    let auth_id = h
        .auth(json!({
            "type": "oauth2_client_credentials",
            "token_url": format!("{}/token", h.provider.uri()),
            "client_id": "cid"
        }))
        .await;
    let tool_id = h.tool("me", &auth_id).await;
    for _ in 0..3 {
        let run = h.run(&tool_id).await;
        assert_eq!(run["response"]["status"], 200, "{run}");
    }
}
