//! Stage 22: admins and members with per-project read/write, enforced on
//! /api routes; users API; safety rails around the last admin.

use std::sync::Arc;

use forge_exec_http::MapSecretResolver;
use forge_store::{Role, Store};
use mcp_forge::login::{cookie_name, hash_password, CSRF_HEADER};
use mcp_forge::{AppState, Security};
use reqwest::header::{COOKIE, LOCATION, SET_COOKIE};
use reqwest::{Method, StatusCode};
use serde_json::{json, Value};
use tokio::net::TcpListener;

const PASSWORD: &str = "correct horse battery";

struct Harness {
    base: String,
    http: reqwest::Client,
    cookie: String,
}

/// A signed-in browser session.
struct As<'a> {
    h: &'a Harness,
    session: String,
}

impl Harness {
    async fn start() -> (Self, Store) {
        let store = Store::open_in_memory().await.unwrap();
        store
            .create_user("root", &hash_password(PASSWORD))
            .await
            .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(mcp_forge::serve(
            listener,
            AppState::new(store.clone(), Arc::new(MapSecretResolver::default()))
                .with_security(Security::default())
                .with_port(addr.port()),
            std::future::pending(),
        ));
        let h = Self {
            base: format!("http://{addr}"),
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            cookie: cookie_name(addr.port()),
        };
        (h, store)
    }

    async fn sign_in(&self, user: &str) -> As<'_> {
        let r = self
            .http
            .post(format!("{}/api/auth/login", self.base))
            .header(CSRF_HEADER, "1")
            .json(&json!({ "username": user, "password": PASSWORD }))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK, "sign in {user}");
        let c = r.headers()[SET_COOKIE].to_str().unwrap().to_owned();
        let session = c
            .strip_prefix(&format!("{}=", self.cookie))
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned();
        As { h: self, session }
    }
}

impl As<'_> {
    async fn call(&self, m: Method, path: &str, body: Value) -> (StatusCode, Value) {
        let r = self
            .h
            .http
            .request(m, format!("{}/api{path}", self.h.base))
            .header(COOKIE, format!("{}={}", self.h.cookie, self.session))
            .header(CSRF_HEADER, "1")
            .json(&body)
            .send()
            .await
            .unwrap();
        let s = r.status();
        (s, r.json().await.unwrap_or(Value::Null))
    }

    async fn ok(&self, m: Method, path: &str, body: Value) -> Value {
        let (s, v) = self.call(m, path, body).await;
        assert!(s.is_success(), "{path}: {s} {v}");
        v
    }

    async fn status(&self, m: Method, path: &str, body: Value) -> u16 {
        self.call(m, path, body).await.0.as_u16()
    }
}

fn tool(name: &str) -> Value {
    json!({ "name": name, "kind": { "type": "http", "method": "GET",
            "base_url": "http://127.0.0.1:9", "path_template": "/x", "params": [] } })
}

#[tokio::test]
async fn members_see_and_change_only_what_they_are_given() {
    let (h, _store) = Harness::start().await;
    let admin = h.sign_in("root").await;

    // Admin sets up two projects and two members.
    let a = admin
        .ok(Method::POST, "/projects", json!({ "name": "Alpha" }))
        .await;
    let b = admin
        .ok(Method::POST, "/projects", json!({ "name": "Beta" }))
        .await;
    let (a, b) = (
        a["id"].as_str().unwrap().to_owned(),
        b["id"].as_str().unwrap().to_owned(),
    );
    let t = admin
        .ok(Method::POST, &format!("/projects/{a}/tools"), tool("ping"))
        .await;
    let tid = t["id"].as_str().unwrap().to_owned();
    let reader = admin
        .ok(
            Method::POST,
            "/users",
            json!({ "username": "rita", "password": PASSWORD, "role": "member" }),
        )
        .await;
    let writer = admin
        .ok(
            Method::POST,
            "/users",
            json!({ "username": "will", "password": PASSWORD, "role": "member" }),
        )
        .await;
    assert_eq!(reader["role"], "member");
    let (rid, wid) = (
        reader["id"].as_str().unwrap().to_owned(),
        writer["id"].as_str().unwrap().to_owned(),
    );
    admin
        .ok(
            Method::PUT,
            &format!("/users/{rid}/access/{a}"),
            json!({ "access": "read" }),
        )
        .await;
    admin
        .ok(
            Method::PUT,
            &format!("/users/{wid}/access/{a}"),
            json!({ "access": "write" }),
        )
        .await;

    let rita = h.sign_in("rita").await;
    let will = h.sign_in("will").await;

    // Project list and status show only granted projects.
    let list = rita.ok(Method::GET, "/projects", Value::Null).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["id"], a.as_str());
    let st = rita.ok(Method::GET, "/auth/status", Value::Null).await;
    assert_eq!(st["user"]["role"], "member");
    assert_eq!(st["access"][&a], "read");
    assert!(admin.ok(Method::GET, "/auth/status", Value::Null).await["access"].is_null());

    // Read: look, don't touch.
    assert_eq!(
        rita.status(Method::GET, &format!("/projects/{a}"), Value::Null)
            .await,
        200
    );
    assert_eq!(
        rita.status(Method::GET, &format!("/projects/{a}/tools"), Value::Null)
            .await,
        200
    );
    assert_eq!(
        rita.status(Method::GET, &format!("/projects/{a}/calls"), Value::Null)
            .await,
        200
    );
    assert_eq!(
        rita.status(
            Method::PUT,
            &format!("/projects/{a}/tools/{tid}"),
            tool("ping2")
        )
        .await,
        403
    );
    assert_eq!(
        rita.status(
            Method::POST,
            &format!("/projects/{a}/tools/{tid}/test"),
            json!({ "arguments": {} })
        )
        .await,
        403,
        "test console calls the real API"
    );
    assert_eq!(
        rita.status(
            Method::POST,
            &format!("/projects/{a}/api-keys"),
            json!({ "name": "k" })
        )
        .await,
        403
    );

    // No access at all.
    assert_eq!(
        rita.status(Method::GET, &format!("/projects/{b}"), Value::Null)
            .await,
        403
    );
    assert_eq!(
        rita.status(Method::GET, &format!("/projects/{b}/tools"), Value::Null)
            .await,
        403
    );

    // Write: change things, but not delete the project or create projects.
    assert_eq!(
        will.status(
            Method::PUT,
            &format!("/projects/{a}/tools/{tid}"),
            tool("ping")
        )
        .await,
        200
    );
    assert_eq!(
        will.status(
            Method::POST,
            &format!("/projects/{a}/tools/{tid}/test"),
            json!({ "arguments": {} })
        )
        .await,
        200
    );
    assert_eq!(
        will.status(
            Method::POST,
            &format!("/projects/{a}/api-keys"),
            json!({ "name": "k" })
        )
        .await,
        201
    );
    assert_eq!(
        will.status(
            Method::PUT,
            &format!("/projects/{a}"),
            json!({ "name": "Alpha renamed" })
        )
        .await,
        200
    );
    assert_eq!(
        will.status(Method::DELETE, &format!("/projects/{a}"), Value::Null)
            .await,
        403
    );
    assert_eq!(
        will.status(Method::POST, "/projects", json!({ "name": "Mine" }))
            .await,
        403
    );

    // Members can't manage users.
    assert_eq!(will.status(Method::GET, "/users", Value::Null).await, 403);
    assert_eq!(
        will.status(
            Method::PUT,
            &format!("/users/{wid}"),
            json!({ "role": "admin" })
        )
        .await,
        403,
        "no self-promotion"
    );

    // People on a project: admins plus members with access.
    let people = rita
        .ok(Method::GET, &format!("/projects/{a}/people"), Value::Null)
        .await;
    let names: Vec<(&str, &Value)> = people
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p["user"]["username"].as_str().unwrap(), &p["access"]))
        .collect();
    assert_eq!(
        names,
        [
            ("rita", &json!("read")),
            ("root", &Value::Null),
            ("will", &json!("write"))
        ]
    );

    // Removing access takes effect at once.
    admin
        .ok(
            Method::PUT,
            &format!("/users/{rid}/access/{a}"),
            json!({ "access": null }),
        )
        .await;
    assert_eq!(
        rita.status(Method::GET, &format!("/projects/{a}"), Value::Null)
            .await,
        403
    );
    let users = admin.ok(Method::GET, "/users", Value::Null).await;
    let w = users
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["username"] == "will")
        .unwrap();
    assert_eq!(w["access"], json!([{ "project_id": a, "access": "write" }]));
}

#[tokio::test]
async fn last_admin_is_protected() {
    let (h, store) = Harness::start().await;
    let admin = h.sign_in("root").await;
    let root_id = store.list_users().await.unwrap()[0].id;

    let (s, v) = admin
        .call(
            Method::PUT,
            &format!("/users/{root_id}"),
            json!({ "role": "member" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(
        admin
            .status(Method::DELETE, &format!("/users/{root_id}"), Value::Null)
            .await,
        409,
        "not yourself"
    );
    assert!(
        store.delete_user("root").await.is_err(),
        "store refuses too"
    );

    // With a second admin, the first can be demoted.
    let second = admin
        .ok(
            Method::POST,
            "/users",
            json!({ "username": "ada", "password": PASSWORD, "role": "admin" }),
        )
        .await;
    admin
        .ok(
            Method::PUT,
            &format!("/users/{root_id}"),
            json!({ "role": "member" }),
        )
        .await;
    assert_eq!(store.get_user(root_id).await.unwrap().role, Role::Member);
    // root is a member now: the users API closes on the next request.
    assert_eq!(admin.status(Method::GET, "/users", Value::Null).await, 403);

    let ada = h.sign_in("ada").await;
    let sid = second["id"].as_str().unwrap();
    assert_eq!(
        ada.status(
            Method::PUT,
            &format!("/users/{sid}"),
            json!({ "role": "member" })
        )
        .await,
        409
    );
    // Password reset by an admin ends the other account's sessions.
    ada.ok(
        Method::PUT,
        &format!("/users/{root_id}/password"),
        json!({ "password": "another long password" }),
    )
    .await;
    assert_eq!(
        admin.status(Method::GET, "/projects", Value::Null).await,
        401
    );
}

#[tokio::test]
async fn app_consent_needs_write_on_the_project() {
    let (h, _store) = Harness::start().await;
    let admin = h.sign_in("root").await;
    let p = admin
        .ok(Method::POST, "/projects", json!({ "name": "Gamma" }))
        .await;
    let pid = p["id"].as_str().unwrap().to_owned();
    let u = admin
        .ok(
            Method::POST,
            "/users",
            json!({ "username": "rita", "password": PASSWORD, "role": "member" }),
        )
        .await;
    admin
        .ok(
            Method::PUT,
            &format!("/users/{}/access/{pid}", u["id"].as_str().unwrap()),
            json!({ "access": "read" }),
        )
        .await;

    let client: Value = h
        .http
        .post(format!("{}/oauth/register", h.base))
        .json(&json!({ "redirect_uris": ["http://127.0.0.1:9/cb"], "token_endpoint_auth_method": "none" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let (_, challenge) = forge_exec_http::oauth::pkce_pair();
    let mut url = reqwest::Url::parse(&format!("{}/oauth/authorize", h.base)).unwrap();
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", client["client_id"].as_str().unwrap())
        .append_pair("redirect_uri", "http://127.0.0.1:9/cb")
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("resource", &format!("{}/mcp/gamma", h.base));
    let r = h.http.get(url).send().await.unwrap();
    let id = r.headers()[LOCATION]
        .to_str()
        .unwrap()
        .strip_prefix("/#/oauth/consent/")
        .unwrap()
        .to_owned();

    let rita = h.sign_in("rita").await;
    assert_eq!(
        rita.status(Method::GET, &format!("/oauth/requests/{id}"), Value::Null)
            .await,
        403
    );
    assert_eq!(
        rita.status(
            Method::POST,
            &format!("/oauth/requests/{id}/approve"),
            json!({})
        )
        .await,
        403
    );
    // The request is still there for someone allowed.
    assert_eq!(
        admin
            .status(Method::GET, &format!("/oauth/requests/{id}"), Value::Null)
            .await,
        200
    );
}
