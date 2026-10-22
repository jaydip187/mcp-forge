//! Stage 13: first-run setup, sign-in, sessions, CSRF header and backoff.

use std::sync::Arc;

use forge_exec_http::MapSecretResolver;
use forge_store::Store;
use mcp_forge::login::{hash_password, token_hash, CSRF_HEADER, SESSION_COOKIE};
use mcp_forge::{AppState, Security};
use reqwest::header::{COOKIE, RETRY_AFTER, SET_COOKIE};
use reqwest::{Client, Response, StatusCode};
use serde_json::{json, Value};
use tokio::net::TcpListener;

const PASSWORD: &str = "correct horse battery";

struct Server {
    base: String,
    state: AppState,
    http: Client,
}

impl Server {
    async fn start(security: Security) -> Self {
        let store = Store::open_in_memory().await.unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let state =
            AppState::new(store, Arc::new(MapSecretResolver::default())).with_security(security);
        tokio::spawn(mcp_forge::serve(
            listener,
            state.clone(),
            std::future::pending(),
        ));
        Self {
            base,
            state,
            http: Client::new(),
        }
    }

    async fn with_user(name: &str) -> Self {
        let s = Self::start(Security::default()).await;
        s.state
            .store
            .create_user(name, &hash_password(PASSWORD))
            .await
            .unwrap();
        s
    }

    fn get(&self, path: &str, session: Option<&str>) -> reqwest::RequestBuilder {
        with_session(self.http.get(format!("{}{path}", self.base)), session)
    }

    /// A POST as the UI sends it: JSON plus the CSRF header.
    fn post(&self, path: &str, body: Value, session: Option<&str>) -> reqwest::RequestBuilder {
        let req = self
            .http
            .post(format!("{}{path}", self.base))
            .header(CSRF_HEADER, "1")
            .json(&body);
        with_session(req, session)
    }

    async fn login(&self, user: &str, password: &str) -> Response {
        self.post(
            "/api/auth/login",
            json!({ "username": user, "password": password }),
            None,
        )
        .send()
        .await
        .unwrap()
    }

    async fn session(&self, user: &str, password: &str) -> String {
        let r = self.login(user, password).await;
        assert_eq!(r.status(), StatusCode::OK);
        session_of(&r)
    }
}

fn with_session(req: reqwest::RequestBuilder, session: Option<&str>) -> reqwest::RequestBuilder {
    match session {
        Some(t) => req.header(COOKIE, format!("{SESSION_COOKIE}={t}")),
        None => req,
    }
}

fn set_cookie(r: &Response) -> String {
    r.headers()
        .get(SET_COOKIE)
        .expect("Set-Cookie")
        .to_str()
        .unwrap()
        .to_owned()
}

fn session_of(r: &Response) -> String {
    let c = set_cookie(r);
    let v = c
        .strip_prefix(&format!("{SESSION_COOKIE}="))
        .expect("session cookie");
    v.split(';').next().unwrap().to_owned()
}

async fn code(r: Response) -> (StatusCode, String) {
    let status = r.status();
    let body: Value = r.json().await.unwrap();
    (
        status,
        body["error"]["code"].as_str().unwrap_or("").to_owned(),
    )
}

#[tokio::test]
async fn first_run_setup_creates_the_first_account() {
    let s = Server::start(Security::default()).await;
    let status: Value = s
        .get("/api/auth/status", None)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["setup_required"], true);
    assert_eq!(status["login_required"], true);
    assert_eq!(status["user"], Value::Null);
    let setup_code = s
        .state
        .login
        .setup_code(&s.state.store)
        .await
        .unwrap()
        .unwrap();

    let r = s.get("/api/projects", None).send().await.unwrap();
    assert_eq!(
        code(r).await,
        (StatusCode::UNAUTHORIZED, "sign_in_required".into())
    );

    let body = |c: &str, pw: &str| json!({ "code": c, "username": "alice", "password": pw });
    let r = s
        .http
        .post(format!("{}/api/auth/setup", s.base))
        .json(&body(&setup_code, PASSWORD))
        .send()
        .await
        .unwrap();
    assert_eq!(
        code(r).await,
        (StatusCode::FORBIDDEN, "csrf".into()),
        "CSRF header needed"
    );
    let r = s
        .post("/api/auth/setup", body("AAAA-BBBB-CCCC", PASSWORD), None)
        .send()
        .await
        .unwrap();
    assert_eq!(
        code(r).await,
        (StatusCode::FORBIDDEN, "invalid_setup_code".into())
    );
    let r = s
        .post("/api/auth/setup", body(&setup_code, "short"), None)
        .send()
        .await
        .unwrap();
    assert_eq!(
        code(r).await,
        (StatusCode::UNPROCESSABLE_ENTITY, "validation".into())
    );

    let r = s
        .post(
            "/api/auth/setup",
            body(&setup_code.to_lowercase(), PASSWORD),
            None,
        )
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CREATED);
    let cookie = set_cookie(&r);
    for attr in ["HttpOnly", "SameSite=Strict", "Path=/", "Max-Age=2592000"] {
        assert!(cookie.contains(attr), "{cookie} lacks {attr}");
    }
    assert!(!cookie.contains("Secure"), "plain HTTP");
    let session = session_of(&r);

    let status: Value = s
        .get("/api/auth/status", Some(&session))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["setup_required"], false);
    assert_eq!(status["user"]["username"], "alice");

    let r = s
        .post("/api/auth/setup", body(&setup_code, PASSWORD), None)
        .send()
        .await
        .unwrap();
    assert_eq!(
        code(r).await,
        (StatusCode::CONFLICT, "conflict".into()),
        "only once"
    );

    let r = s.get("/api/projects", Some(&session)).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let r = s
        .http
        .post(format!("{}/api/projects", s.base))
        .header(COOKIE, format!("{SESSION_COOKIE}={session}"))
        .json(&json!({ "name": "Demo" }))
        .send()
        .await
        .unwrap();
    assert_eq!(code(r).await, (StatusCode::FORBIDDEN, "csrf".into()));
    let r = s
        .post("/api/projects", json!({ "name": "Demo" }), Some(&session))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn login_and_logout() {
    let s = Server::with_user("alice").await;
    assert!(s
        .state
        .login
        .setup_code(&s.state.store)
        .await
        .unwrap()
        .is_none());

    let wrong: Value = s
        .login("alice", "wrong password")
        .await
        .json()
        .await
        .unwrap();
    let unknown: Value = s.login("nobody", PASSWORD).await.json().await.unwrap();
    assert_eq!(wrong["error"]["code"], "invalid_credentials");
    assert_eq!(wrong, unknown, "no hint whether the user exists");

    let session = s.session("ALICE", PASSWORD).await;
    let r = s.get("/api/projects", Some(&session)).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);

    let r = s
        .post("/api/auth/logout", json!({}), Some(&session))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    assert!(set_cookie(&r).contains("Max-Age=0"));
    let r = s.get("/api/projects", Some(&session)).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);

    let r = s
        .get("/api/projects", Some("not-a-token"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn repeated_failures_lock_the_account_out() {
    let s = Server::with_user("alice").await;
    for _ in 0..5 {
        assert_eq!(
            s.login("alice", "wrong password").await.status(),
            StatusCode::UNAUTHORIZED
        );
    }
    let r = s.login("alice", PASSWORD).await;
    assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
    let wait: u64 = r.headers()[RETRY_AFTER].to_str().unwrap().parse().unwrap();
    assert!((1..=30).contains(&wait));
}

#[tokio::test]
async fn idle_sessions_expire() {
    let s = Server::with_user("alice").await;
    let session = s.session("alice", PASSWORD).await;
    let stale = chrono::Utc::now() - chrono::Duration::days(8);
    s.state
        .store
        .touch_session(&token_hash(&session), stale)
        .await
        .unwrap();
    let r = s.get("/api/projects", Some(&session)).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    assert!(s
        .state
        .store
        .find_session(&token_hash(&session))
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn password_change_ends_other_sessions() {
    let s = Server::with_user("alice").await;
    let a = s.session("alice", PASSWORD).await;
    let b = s.session("alice", PASSWORD).await;
    let change = |current: &str| json!({ "current_password": current, "new_password": "a brand new password" });
    let r = s
        .post("/api/auth/password", change("nope"), Some(&a))
        .send()
        .await
        .unwrap();
    assert_eq!(
        code(r).await,
        (StatusCode::UNPROCESSABLE_ENTITY, "validation".into())
    );
    let r = s
        .post("/api/auth/password", change(PASSWORD), Some(&a))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);

    assert_eq!(
        s.get("/api/projects", Some(&a))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        s.get("/api/projects", Some(&b))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        s.login("alice", PASSWORD).await.status(),
        StatusCode::UNAUTHORIZED
    );
    s.session("alice", "a brand new password").await;
}

#[tokio::test]
async fn admin_token_still_opens_api_and_guards_mcp() {
    const TOKEN: &str = "test-admin-token-123";
    let s = Server::start(Security {
        admin_token: Some(TOKEN.into()),
        loopback_hosts_only: false,
        require_login: true,
    })
    .await;
    let bearer = format!("Bearer {TOKEN}");
    let r = s.get("/api/projects", None).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    let r = s
        .get("/api/projects", None)
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    // Scripts using the token don't need the CSRF header.
    let r = s
        .http
        .post(format!("{}/api/projects", s.base))
        .header("Authorization", &bearer)
        .json(&json!({ "name": "Scripted" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CREATED);
    let r = s
        .http
        .post(format!("{}/mcp/scripted", s.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    // The UI shell stays public; it shows the sign-in screen.
    let r = s.get("/", None).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
}
