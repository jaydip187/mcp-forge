use super::*;
use serde_json::json;

fn tool(method: &str, path: &str, tags: &[&str], enabled: bool) -> ToolDef {
    serde_json::from_value(json!({
            "id": "00000000-0000-0000-0000-000000000001",
            "project_id": "00000000-0000-0000-0000-000000000002",
            "name": "t", "description": "", "enabled": enabled, "tags": tags,
            "kind": { "type": "http", "method": method, "base_url": "https://x.io", "path_template": path },
            "source": { "type": "manual" }, "overrides": {}, "response_shape": null,
            "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z"
        }))
        .unwrap()
}

#[test]
fn globs_match_like_the_ui() {
    assert!(path_matches("/pet/*", "/pet/{petId}"));
    assert!(!path_matches("/pet/*", "/pet/{petId}/uploadImage"));
    assert!(path_matches("/pet/**", "/pet/{petId}/uploadImage"));
    assert!(path_matches(
        "/repos/*/*/issues",
        "/repos/{owner}/{repo}/issues"
    ));
    assert!(path_matches("store", "/store/inventory"));
    assert!(path_matches("STORE", "/store/inventory"));
    assert!(path_matches("/a?c", "/abc"));
    assert!(!path_matches("/a?c", "/a/c"));
    assert!(path_matches("", "/anything"));
}

#[test]
fn exposure_rules() {
    let s = ProjectSettings::default();
    assert!(s.exposes(&tool("POST", "/a", &[], true)));
    assert_eq!(
        s.hidden_reason(&tool("GET", "/a", &[], false)),
        Some(Hidden::Disabled)
    );

    let ro = ProjectSettings {
        read_only: true,
        ..Default::default()
    };
    assert!(ro.exposes(&tool("GET", "/a", &[], true)));
    assert_eq!(
        ro.hidden_reason(&tool("DELETE", "/a", &[], true)),
        Some(Hidden::ReadOnly)
    );

    let tags = ProjectSettings {
        include_tags: vec!["pet".into()],
        exclude_tags: vec!["Admin".into()],
        ..Default::default()
    };
    assert!(tags.exposes(&tool("GET", "/a", &["pet"], true)));
    assert_eq!(
        tags.hidden_reason(&tool("GET", "/a", &["store"], true)),
        Some(Hidden::Tag)
    );
    assert_eq!(
        tags.hidden_reason(&tool("GET", "/a", &["pet", "admin"], true)),
        Some(Hidden::Tag)
    );
    assert_eq!(
        tags.hidden_reason(&tool("GET", "/a", &[], true)),
        Some(Hidden::Tag)
    );

    let paths = ProjectSettings {
        include_paths: vec!["/repos/**".into()],
        exclude_paths: vec!["/repos/*/*/hooks/**".into()],
        ..Default::default()
    };
    assert!(paths.exposes(&tool("GET", "/repos/{o}/{r}/issues", &[], true)));
    assert_eq!(
        paths.hidden_reason(&tool("GET", "/user", &[], true)),
        Some(Hidden::Path)
    );
    assert_eq!(
        paths.hidden_reason(&tool("GET", "/repos/{o}/{r}/hooks/{id}", &[], true)),
        Some(Hidden::Path)
    );
}

#[test]
fn validation_and_defaults() {
    let s: ProjectSettings = serde_json::from_value(json!({})).unwrap();
    assert_eq!(s, ProjectSettings::default());
    assert_eq!(s.max_response_chars, DEFAULT_MAX_RESPONSE_CHARS);
    let bad = ProjectSettings {
        max_response_chars: 10,
        include_tags: vec![" ".into()],
        ..Default::default()
    };
    let e = bad.validate().unwrap_err();
    assert!(e.has("max_response_chars") && e.has("include_tags[0]"));
}
