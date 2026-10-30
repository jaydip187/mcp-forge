use super::*;
use serde_json::json;

#[test]
fn secret_names() {
    for n in [
        "password",
        "X-Api-Key",
        "apiKey",
        "access_token",
        "Authorization",
        "client_secret",
    ] {
        assert!(is_secret_name(n), "{n}");
    }
    for n in ["owner", "repo", "query", "limit", "author"] {
        assert!(!is_secret_name(n), "{n}");
    }
}

#[test]
fn arguments_are_redacted_at_any_depth() {
    let args = json!({
        "owner": "rust-lang",
        "password": "hunter2",
        "body": { "items": [{ "api_key": "k", "name": "a" }], "token": null },
    });
    let logged = logged_arguments(args.as_object().unwrap());
    assert_eq!(
        logged,
        json!({
            "owner": "rust-lang",
            "password": REDACTED,
            "body": { "items": [{ "api_key": REDACTED, "name": "a" }], "token": null },
        })
    );
}

#[test]
fn long_arguments_are_cut() {
    let args = json!({ "q": "x".repeat(10_000) });
    let Value::String(s) = logged_arguments(args.as_object().unwrap()) else {
        panic!("cut arguments become a string");
    };
    assert_eq!(s.chars().count(), MAX_LOGGED_ARGS_CHARS + 1);
    assert!(s.ends_with('…'));
}

#[test]
fn errors_are_cut() {
    assert_eq!(logged_error("short"), "short");
    let long = logged_error(&"é".repeat(5_000));
    assert_eq!(long.chars().count(), MAX_LOGGED_ERROR_CHARS + 1);
}

#[test]
fn status_round_trips() {
    for s in [CallStatus::Ok, CallStatus::Error, CallStatus::UnknownTool] {
        assert_eq!(CallStatus::parse(s.as_str()), Some(s));
        assert_eq!(serde_json::to_value(s).unwrap(), json!(s.as_str()));
    }
    assert_eq!(CallStatus::parse("nope"), None);
}
