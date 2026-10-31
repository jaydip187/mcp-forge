use super::*;

#[test]
fn redirect_uris() {
    assert!(redirect_uri_ok("https://claude.ai/api/mcp/auth_callback"));
    assert!(redirect_uri_ok("http://localhost:6274/oauth/callback"));
    assert!(redirect_uri_ok("http://127.0.0.1:33418/cb"));
    assert!(redirect_uri_ok(
        "cursor://anysphere.cursor-mcp/oauth/callback"
    ));
    assert!(
        !redirect_uri_ok("http://evil.example.com/cb"),
        "plain http off loopback"
    );
    assert!(!redirect_uri_ok("https://example.com/cb#frag"));
    assert!(!redirect_uri_ok("javascript:alert(1)"));
    assert!(!redirect_uri_ok("not a url"));
}

#[test]
fn resource_names_a_project_on_this_server() {
    let o = "https://forge.example.com";
    assert_eq!(
        resource_slug("https://forge.example.com/mcp/demo", o),
        Some("demo")
    );
    assert_eq!(
        resource_slug("https://forge.example.com/mcp/demo/", o),
        Some("demo")
    );
    assert_eq!(resource_slug("https://other.example.com/mcp/demo", o), None);
    assert_eq!(resource_slug("https://forge.example.com/api/demo", o), None);
    assert_eq!(resource_slug("https://forge.example.com/mcp/a/b", o), None);
    assert_eq!(resource_slug("https://forge.example.com/mcp/", o), None);
}

#[test]
fn basic_credentials_decode() {
    // base64("mcpc_x:s%3Aecret") → secret with a form-encoded colon
    assert_eq!(
        decode_basic("bWNwY194OnMlM0FlY3JldA=="),
        Some(("mcpc_x".into(), "s:ecret".into()))
    );
    assert_eq!(decode_basic("!!!"), None);
}

#[test]
fn origin_follows_the_request() {
    let mut h = HeaderMap::new();
    h.insert(HOST, "127.0.0.1:7410".parse().unwrap());
    assert_eq!(origin(&h), "http://127.0.0.1:7410");
    h.insert("x-forwarded-proto", "https".parse().unwrap());
    assert_eq!(origin(&h), "https://127.0.0.1:7410");
    assert_eq!(
        resource_metadata_url(&h, "demo"),
        "https://127.0.0.1:7410/.well-known/oauth-protected-resource/mcp/demo"
    );
}
