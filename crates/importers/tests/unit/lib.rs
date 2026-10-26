use super::*;

#[test]
fn sanitizes() {
    assert_eq!(
        sanitize_name("repos/get-content", false),
        "repos_get-content"
    );
    assert_eq!(sanitize_name("filter[name]", true), "filter_name");
    assert_eq!(sanitize_name("$top", true), "top");
    assert_eq!(sanitize_name("a.b", true), "a.b");
    assert_eq!(sanitize_name("a.b", false), "a_b");
    assert_eq!(sanitize_name("get /pets/{petId}", false), "get_pets_petId");
    assert_eq!(sanitize_name("日本", true), "");
    assert_eq!(sanitize_name(&"x".repeat(100), false).len(), 64);
}

#[test]
fn dedupes() {
    let mut taken = HashSet::new();
    assert_eq!(unique_name("a".into(), &mut taken), "a");
    assert_eq!(unique_name("a".into(), &mut taken), "a_2");
    assert_eq!(unique_name("a".into(), &mut taken), "a_3");
    let long = "y".repeat(64);
    assert_eq!(unique_name(long.clone(), &mut taken), long);
    let second = unique_name(long, &mut taken);
    assert_eq!(second.len(), 64);
    assert!(second.ends_with("_2"));
}

#[test]
fn parses_json_and_yaml() {
    let j = parse_document("{\"openapi\":\"3.1.0\"}").unwrap();
    assert_eq!(j["openapi"], "3.1.0");
    let y = parse_document("openapi: 3.0.0\npaths:\n  /a:\n    get:\n      responses:\n        200:\n          description: ok\n").unwrap();
    assert_eq!(
        y["paths"]["/a"]["get"]["responses"]["200"]["description"],
        "ok"
    );
    assert!(parse_document("just a string").is_err());
    assert!(parse_document("{ nope").is_err());
}
