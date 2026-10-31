//! Stage 21: mcp-forge's own OAuth server for /mcp — discovery, dynamic
//! registration, authorize → consent → code → token with PKCE, refresh
//! rotation, revocation, connected apps, and the errors along the way.

use std::collections::HashMap;
use std::sync::Arc;

use forge_exec_http::oauth::pkce_pair;
use forge_exec_http::MapSecretResolver;
use forge_store::Store;
use mcp_forge::{AppState, Security};
use reqwest::header::{LOCATION, WWW_AUTHENTICATE};
use reqwest::{StatusCode, Url};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::{ClientHandler, ServiceExt};
use serde_json::{json, Value};
use tokio::net::TcpListener;

/// reqwest's `form` feature is off in this workspace.
fn form_body(pairs: &[(&str, &str)]) -> String {
    let mut s = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in pairs {
        s.append_pair(k, v);
    }
    s.finish()
}

const REDIRECT: &str = "http://127.0.0.1:9/callback";

#[derive(Clone)]
struct Client;
impl ClientHandler for Client {}

struct Harness {
    base: String,
    http: reqwest::Client,
}

impl Harness {
    async fn start(security: Security) -> Self {
        let store = Store::open_in_memory().await.unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(mcp_forge::serve(
            listener,
            AppState::new(store, Arc::new(MapSecretResolver::default())).with_security(security),
            std::future::pending(),
        ));
        let h = Self {
            base,
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
        };
        for name in ["demo", "other"] {
            let p = h
                .json(
                    reqwest::Method::POST,
                    "/api/projects",
                    json!({ "name": name }),
                )
                .await;
            let mut settings = p["settings"].clone();
            settings["require_key"] = json!(true);
            let pid = p["id"].as_str().unwrap();
            h.json(
                reqwest::Method::PUT,
                &format!("/api/projects/{pid}/settings"),
                settings,
            )
            .await;
            h.json(
                reqwest::Method::POST,
                &format!("/api/projects/{pid}/tools"),
                json!({ "name": "ping", "kind": { "type": "http", "method": "GET",
                        "base_url": "http://127.0.0.1:9", "path_template": "/ping", "params": [] } }),
            )
            .await;
        }
        h
    }

    fn open() -> Security {
        Security {
            require_login: false,
            ..Security::default()
        }
    }

    async fn json(&self, m: reqwest::Method, path: &str, body: Value) -> Value {
        let r = self
            .http
            .request(m, format!("{}{path}", self.base))
            .header("x-mcp-forge", "1")
            .json(&body)
            .send()
            .await
            .unwrap();
        assert!(
            r.status().is_success(),
            "{path}: {}",
            r.text().await.unwrap()
        );
        r.json().await.unwrap_or(Value::Null)
    }

    async fn register(&self, method: &str) -> Value {
        let r = self
            .http
            .post(format!("{}/oauth/register", self.base))
            .json(&json!({
                "client_name": "Test client",
                "redirect_uris": [REDIRECT],
                "token_endpoint_auth_method": method,
                "grant_types": ["authorization_code", "refresh_token"],
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::CREATED);
        r.json().await.unwrap()
    }

    fn authorize_url(&self, client_id: &str, challenge: &str, resource: Option<&str>) -> String {
        let mut u = Url::parse(&format!("{}/oauth/authorize", self.base)).unwrap();
        {
            let mut q = u.query_pairs_mut();
            q.append_pair("response_type", "code")
                .append_pair("client_id", client_id)
                .append_pair("redirect_uri", REDIRECT)
                .append_pair("state", "st-1")
                .append_pair("code_challenge", challenge)
                .append_pair("code_challenge_method", "S256");
            if let Some(r) = resource {
                q.append_pair("resource", r);
            }
        }
        u.to_string()
    }

    /// authorize → consent page id.
    async fn consent_id(&self, client_id: &str, challenge: &str, slug: &str) -> String {
        let resource = format!("{}/mcp/{slug}", self.base);
        let r = self
            .http
            .get(self.authorize_url(client_id, challenge, Some(&resource)))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::SEE_OTHER);
        let loc = r.headers()[LOCATION].to_str().unwrap().to_owned();
        loc.strip_prefix("/#/oauth/consent/")
            .expect("consent page")
            .to_owned()
    }

    /// Approves a consent request; returns the code.
    async fn approve(&self, id: &str) -> String {
        let r = self
            .json(
                reqwest::Method::POST,
                &format!("/api/oauth/requests/{id}/approve"),
                json!({}),
            )
            .await;
        let back = Url::parse(r["redirect"].as_str().unwrap()).unwrap();
        assert!(back.as_str().starts_with(REDIRECT));
        let q: HashMap<String, String> = back.query_pairs().into_owned().collect();
        assert_eq!(q["state"], "st-1");
        assert_eq!(q["iss"], self.base);
        q["code"].clone()
    }

    async fn token(
        &self,
        form: &[(&str, &str)],
        basic: Option<(&str, &str)>,
    ) -> (StatusCode, Value) {
        let mut req = self
            .http
            .post(format!("{}/oauth/token", self.base))
            .header("content-type", "application/x-www-form-urlencoded")
            .body(form_body(form));
        if let Some((id, secret)) = basic {
            req = req.basic_auth(id, Some(secret));
        }
        let r = req.send().await.unwrap();
        let status = r.status();
        (status, r.json().await.unwrap_or(Value::Null))
    }

    async fn mcp_status(&self, slug: &str, bearer: Option<&str>) -> reqwest::Response {
        let mut req = self
            .http
            .post(format!("{}/mcp/{slug}", self.base))
            .header("accept", "application/json, text/event-stream")
            .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": { "protocolVersion": "2025-06-18", "capabilities": {},
                            "clientInfo": { "name": "t", "version": "1" } } }));
        if let Some(b) = bearer {
            req = req.bearer_auth(b);
        }
        req.send().await.unwrap()
    }
}

#[tokio::test]
async fn sign_in_end_to_end() {
    let h = Harness::start(Harness::open()).await;

    // Discovery starts from a 401.
    let r = h.mcp_status("demo", None).await;
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    let www = r.headers()[WWW_AUTHENTICATE].to_str().unwrap().to_owned();
    let prm_url = format!("{}/.well-known/oauth-protected-resource/mcp/demo", h.base);
    assert_eq!(www, format!("Bearer resource_metadata=\"{prm_url}\""));
    let prm: Value = h
        .http
        .get(&prm_url)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(prm["resource"], format!("{}/mcp/demo", h.base));
    assert_eq!(prm["authorization_servers"], json!([h.base]));
    let asm: Value = h
        .http
        .get(format!("{}/.well-known/oauth-authorization-server", h.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(asm["issuer"], h.base);
    assert_eq!(asm["code_challenge_methods_supported"], json!(["S256"]));
    assert_eq!(
        asm["registration_endpoint"],
        format!("{}/oauth/register", h.base)
    );
    let missing: StatusCode = h
        .http
        .get(format!(
            "{}/.well-known/oauth-protected-resource/mcp/nope",
            h.base
        ))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(missing, StatusCode::NOT_FOUND);

    // Register a public client, sign in, approve.
    let client = h.register("none").await;
    let cid = client["client_id"].as_str().unwrap().to_owned();
    assert!(client.get("client_secret").is_none());
    let (verifier, challenge) = pkce_pair();
    let id = h.consent_id(&cid, &challenge, "demo").await;
    let info = h
        .json(
            reqwest::Method::GET,
            &format!("/api/oauth/requests/{id}"),
            Value::Null,
        )
        .await;
    assert_eq!(info["client_name"], "Test client");
    assert_eq!(info["project"]["slug"], "demo");
    assert_eq!(info["redirect_host"], "127.0.0.1");
    let code = h.approve(&id).await;

    // Wrong verifier: refused (and the code is spent).
    let (s, e) = h
        .token(
            &[
                ("grant_type", "authorization_code"),
                ("code", &code),
                ("client_id", &cid),
                ("redirect_uri", REDIRECT),
                ("code_verifier", &pkce_pair().0),
            ],
            None,
        )
        .await;
    assert_eq!(
        (s, e["error"].as_str()),
        (StatusCode::BAD_REQUEST, Some("invalid_grant"))
    );
    let (s, _) = h
        .token(
            &[
                ("grant_type", "authorization_code"),
                ("code", &code),
                ("client_id", &cid),
                ("redirect_uri", REDIRECT),
                ("code_verifier", &verifier),
            ],
            None,
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "codes are single use");

    // Again, properly.
    let (verifier, challenge) = pkce_pair();
    let code = h
        .approve(&h.consent_id(&cid, &challenge, "demo").await)
        .await;
    let (s, t) = h
        .token(
            &[
                ("grant_type", "authorization_code"),
                ("code", &code),
                ("client_id", &cid),
                ("redirect_uri", REDIRECT),
                ("code_verifier", &verifier),
            ],
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{t}");
    assert_eq!(t["token_type"], "Bearer");
    assert_eq!(t["expires_in"], 3600);
    let access = t["access_token"].as_str().unwrap().to_owned();
    let refresh = t["refresh_token"].as_str().unwrap().to_owned();
    assert!(access.starts_with("mcpo_") && refresh.starts_with("mcpr_"));

    // The token opens this project's /mcp, not another's.
    assert_eq!(
        h.mcp_status("demo", Some(&access)).await.status(),
        StatusCode::OK
    );
    assert_eq!(
        h.mcp_status("other", Some(&access)).await.status(),
        StatusCode::UNAUTHORIZED
    );
    let config = StreamableHttpClientTransportConfig::with_uri(format!("{}/mcp/demo", h.base))
        .auth_header(access.clone());
    let c = Client
        .serve(StreamableHttpClientTransport::from_config(config))
        .await
        .unwrap();
    let tools = c.list_all_tools().await.unwrap();
    assert_eq!(tools.len(), 1);
    let _ = c
        .call_tool(rmcp::model::CallToolRequestParams::new("ping"))
        .await;
    c.cancel().await.unwrap();

    // Connected apps list it; the call log names it.
    let pid = h
        .json(reqwest::Method::GET, "/api/projects", Value::Null)
        .await[0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let apps = h
        .json(
            reqwest::Method::GET,
            &format!("/api/projects/{pid}/connected-apps"),
            Value::Null,
        )
        .await;
    assert_eq!(apps[0]["client_name"], "Test client");
    assert!(apps[0].get("access_hash").is_none());
    let calls = h
        .json(
            reqwest::Method::GET,
            &format!("/api/projects/{pid}/calls"),
            Value::Null,
        )
        .await;
    assert_eq!(calls[0]["api_key"], "Test client (sign-in)");

    // Refresh rotates both tokens.
    let (s, t2) = h
        .token(
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", &refresh),
                ("client_id", &cid),
            ],
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{t2}");
    let access2 = t2["access_token"].as_str().unwrap().to_owned();
    assert_ne!(access2, access);
    assert_eq!(
        h.mcp_status("demo", Some(&access)).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        h.mcp_status("demo", Some(&access2)).await.status(),
        StatusCode::OK
    );
    let (s, _) = h
        .token(
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", &refresh),
                ("client_id", &cid),
            ],
            None,
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "old refresh token is spent");

    // Revocation (RFC 7009) and removal from the UI.
    let r = h
        .http
        .post(format!("{}/oauth/revoke", h.base))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(form_body(&[
            ("token", access2.as_str()),
            ("client_id", cid.as_str()),
        ]))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(
        h.mcp_status("demo", Some(&access2)).await.status(),
        StatusCode::UNAUTHORIZED
    );
    let apps = h
        .json(
            reqwest::Method::GET,
            &format!("/api/projects/{pid}/connected-apps"),
            Value::Null,
        )
        .await;
    assert_eq!(apps, json!([]));
}

#[tokio::test]
async fn confidential_clients_and_bad_requests() {
    let h = Harness::start(Harness::open()).await;
    let client = h.register("client_secret_basic").await;
    let cid = client["client_id"].as_str().unwrap().to_owned();
    let secret = client["client_secret"].as_str().unwrap().to_owned();

    let (verifier, challenge) = pkce_pair();
    let code = h
        .approve(&h.consent_id(&cid, &challenge, "demo").await)
        .await;
    let form = [
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("redirect_uri", REDIRECT),
        ("code_verifier", verifier.as_str()),
    ];
    let (s, e) = h
        .token(&[&form[..], &[("client_id", cid.as_str())]].concat(), None)
        .await;
    assert_eq!(
        (s, e["error"].as_str()),
        (StatusCode::UNAUTHORIZED, Some("invalid_client"))
    );
    let (s, _) = h.token(&form, Some((&cid, "wrong"))).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, t) = h.token(&form, Some((&cid, &secret))).await;
    assert_eq!(s, StatusCode::OK, "{t}");

    // Unknown client / mismatched redirect: an error page, never a redirect.
    let r = h
        .http
        .get(h.authorize_url("mcpc_nope", &challenge, None))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    let bad_redirect = h
        .authorize_url(&cid, &challenge, None)
        .replace("callback", "elsewhere");
    let r = h.http.get(bad_redirect).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);

    // No resource: back to the client with invalid_target.
    let r = h
        .http
        .get(h.authorize_url(&cid, &challenge, None))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    assert!(r.headers()[LOCATION]
        .to_str()
        .unwrap()
        .contains("error=invalid_target"));

    // Denied consent.
    let id = h.consent_id(&cid, &challenge, "demo").await;
    let r = h
        .json(
            reqwest::Method::POST,
            &format!("/api/oauth/requests/{id}/deny"),
            json!({}),
        )
        .await;
    let back = r["redirect"].as_str().unwrap();
    assert!(
        back.contains("error=access_denied") && back.contains("state=st-1"),
        "{back}"
    );

    // Registration rules.
    for body in [
        json!({ "redirect_uris": [] }),
        json!({ "redirect_uris": ["http://evil.example.com/cb"] }),
        json!({ "redirect_uris": [REDIRECT], "token_endpoint_auth_method": "private_key_jwt" }),
        json!({ "redirect_uris": [REDIRECT], "grant_types": ["client_credentials"] }),
    ] {
        let r = h
            .http
            .post(format!("{}/oauth/register", h.base))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::BAD_REQUEST, "{body}");
    }
    let (s, e) = h
        .token(
            &[("grant_type", "password"), ("client_id", &cid)],
            Some((&cid, &secret)),
        )
        .await;
    assert_eq!(
        (s, e["error"].as_str()),
        (StatusCode::BAD_REQUEST, Some("unsupported_grant_type"))
    );
}

#[tokio::test]
async fn consent_needs_a_signed_in_user() {
    // Sign-in on: the consent API refuses calls without a session.
    let store = Store::open_in_memory().await.unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(mcp_forge::serve(
        listener,
        AppState::new(store, Arc::new(MapSecretResolver::default())),
        std::future::pending(),
    ));
    let r = reqwest::Client::new()
        .post(format!("{base}/api/oauth/requests/whatever/approve"))
        .header("x-mcp-forge", "1")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
}
