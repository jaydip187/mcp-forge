use super::*;

#[test]
fn infers_types_not_values() {
    let s = infer_schema(&json!({
        "name": "Ada", "age": 36, "score": 1.5, "admin": false, "tags": ["x"], "boss": null,
        "address": { "city": "London" }
    }));
    assert_eq!(s["properties"]["name"], json!({ "type": "string" }));
    assert_eq!(s["properties"]["age"], json!({ "type": "integer" }));
    assert_eq!(s["properties"]["score"], json!({ "type": "number" }));
    assert_eq!(
        s["properties"]["tags"],
        json!({ "type": "array", "items": { "type": "string" } })
    );
    assert_eq!(
        s["properties"]["address"]["properties"]["city"],
        json!({ "type": "string" })
    );
    assert!(!s.to_string().contains("Ada"), "no example values");
}

#[test]
fn bodies() {
    assert!(matches!(body_from("", None), Body::None));
    assert!(matches!(body_from(r#"{"a":1}"#, None), Body::Json(_)));
    assert!(matches!(
        body_from(r#"{"a":1}"#, Some("application/json; charset=utf-8")),
        Body::Json(_)
    ));
    match body_from("a=1&b=2", None) {
        Body::Other(m) => assert_eq!(m, "application/x-www-form-urlencoded"),
        other => panic!("{other:?}"),
    }
    match body_from("<x/>", Some("application/xml")) {
        Body::Other(m) => assert_eq!(m, "application/xml"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn secrets_never_become_defaults() {
    assert_eq!(
        sample_param("page", Some("2"), true).default.as_deref(),
        Some("2")
    );
    assert_eq!(sample_param("api_key", Some("abc"), true).default, None);
    assert_eq!(sample_param("page", Some("2"), false).default, None);
    assert_eq!(sample_param("q", Some("{{term}}"), true).default, None);
}

#[test]
fn schemes_from_headers() {
    let mut c = Collection::default();
    assert_eq!(
        c.scheme_for_header("Authorization", "Bearer abc")
            .as_deref(),
        Some("bearerAuth")
    );
    assert_eq!(
        c.scheme_for_header("Authorization", "Basic YTpi")
            .as_deref(),
        Some("basicAuth")
    );
    assert_eq!(
        c.scheme_for_header("X-Api-Key", "k").as_deref(),
        Some("header_X_Api_Key")
    );
    assert_eq!(c.scheme_for_header("X-Request-Id", "1"), None);
    assert_eq!(
        c.scheme_for_header("Cookie", "session=abc"),
        None,
        "cookies aren't auth to suggest"
    );
    assert_eq!(
        c.schemes["header_X_Api_Key"],
        json!({ "type": "apiKey", "in": "header", "name": "X-Api-Key" })
    );
}

#[test]
fn builds_servers_and_dedupes() {
    let op = |m: &str, base: &str, path: &str| Op {
        method: m.into(),
        base: Some(base.into()),
        path: path.into(),
        ..Op::default()
    };
    let c = Collection {
        title: "t".into(),
        ops: vec![
            op("GET", "https://a.example.com", "/x"),
            op("GET", "https://a.example.com", "/x"),
            op("POST", "https://a.example.com", "/x"),
            op("GET", "https://b.example.com", "/y"),
        ],
        ..Collection::default()
    };
    let doc = c.into_openapi("har");
    assert_eq!(doc["servers"], json!([{ "url": "https://a.example.com" }]));
    assert_eq!(doc["info"][ORIGIN_KEY], "har");
    assert!(doc["paths"]["/x"]["get"].is_object() && doc["paths"]["/x"]["post"].is_object());
    assert_eq!(
        doc["paths"]["/y"]["get"]["servers"],
        json!([{ "url": "https://b.example.com" }])
    );
    assert!(doc["paths"]["/x"]["get"].get("servers").is_none());
}

#[test]
fn ids_and_words() {
    use crate::har::looks_like_id;
    for id in [
        "42",
        "3f2504e0-4f89-11d3-9a0c-0305e82c3301",
        "5f8d0d55b54764421b7156c3",
        "cus_9s6XKzkNRiz8i3abc123",
    ] {
        assert!(looks_like_id(id), "{id}");
    }
    for word in ["users", "v1", "me", "search", "2fa", "deadbeef"] {
        assert!(!looks_like_id(word), "{word}");
    }
}

#[test]
fn shell_words() {
    use crate::curl::words;
    assert_eq!(
        words("curl -H 'X-A: b c' \"q\\\"x\" $'a\\nb' plain\\ word").unwrap(),
        ["curl", "-H", "X-A: b c", "q\"x", "a\nb", "plain word"]
    );
    assert_eq!(
        words("curl \\\n  -X POST ^\r\n  url `\n -s").unwrap(),
        ["curl", "-X", "POST", "url", "-s"]
    );
    assert!(words("curl 'open").is_err());
}
