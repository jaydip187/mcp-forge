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
