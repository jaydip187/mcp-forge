use super::*;

#[test]
fn deserializes_and_validates() {
    let input: AuthConfigInput = serde_json::from_value(serde_json::json!({
        "name": "github",
        "kind": { "type": "api_key", "location": "header", "name": "X-Api-Key" },
        "secret_ref": "env:GH_KEY"
    }))
    .unwrap();
    input.validate().unwrap();

    let basic = AuthConfigInput {
        name: "b".into(),
        kind: AuthKind::Basic {
            username: "a:b".into(),
        },
        secret_ref: SecretRef::parse("env:P").unwrap(),
    };
    assert!(basic.validate().unwrap_err().has("kind.username"));

    let key = AuthConfigInput {
        name: "bad name".into(),
        kind: AuthKind::ApiKey {
            location: ApiKeyLocation::Query,
            name: "has space".into(),
        },
        secret_ref: SecretRef::parse("env:P").unwrap(),
    };
    let e = key.validate().unwrap_err();
    assert!(e.has("name") && e.has("kind.name"));
}

#[test]
fn raw_secret_rejected_on_deserialize() {
    let r = serde_json::from_value::<AuthConfigInput>(serde_json::json!({
        "name": "x", "kind": { "type": "bearer" }, "secret_ref": "ghp_raw_token"
    }));
    assert!(r.is_err());
}

#[test]
fn oauth_kinds_validate() {
    let parse = |kind: serde_json::Value| -> AuthConfigInput {
        serde_json::from_value(
            serde_json::json!({ "name": "o", "kind": kind, "secret_ref": "env:SECRET" }),
        )
        .unwrap()
    };
    let cc = parse(serde_json::json!({
        "type": "oauth2_client_credentials",
        "token_url": "https://auth.example.com/oauth/token",
        "client_id": "abc",
        "scopes": ["read:users", "write"],
        "audience": "https://api.example.com"
    }));
    cc.validate().unwrap();
    assert!(cc.kind.is_oauth());
    let AuthKind::OAuth2ClientCredentials { client_auth, .. } = &cc.kind else {
        panic!("client credentials");
    };
    assert_eq!(*client_auth, OAuthClientAuth::Body, "default");

    let code = parse(serde_json::json!({
        "type": "oauth2_authorization_code",
        "auth_url": "http://localhost:9000/authorize",
        "token_url": "http://127.0.0.1:9000/token",
        "client_id": "abc",
        "client_auth": "basic"
    }));
    code.validate().unwrap();

    let bad = parse(serde_json::json!({
        "type": "oauth2_authorization_code",
        "auth_url": "http://auth.example.com/authorize",
        "token_url": "https://user:pw@auth.example.com/token",
        "client_id": " ",
        "scopes": ["has space"]
    }));
    let e = bad.validate().unwrap_err();
    for f in [
        "kind.auth_url",
        "kind.token_url",
        "kind.client_id",
        "kind.scopes[0]",
    ] {
        assert!(e.has(f), "{f}: {e:?}");
    }
    assert!(!AuthKind::Bearer.is_oauth());
}
