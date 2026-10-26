use super::*;

#[test]
fn rejects_raw_secrets_and_bad_versions() {
    let raw = "version: 1\nauth_configs:\n- name: gh\n  kind: {type: bearer}\n  secret_ref: ghp_supersecret\n";
    let e = parse_bundle(raw).unwrap_err();
    assert!(e.to_string().contains("not a raw secret"), "{e}");
    assert!(parse_bundle("version: 9\n").unwrap_err().has("version"));
    assert!(parse_bundle("not: [valid").unwrap_err().has("content"));
}

#[test]
fn tool_auth_by_name() {
    let b = parse_bundle(
            "version: 1\ntools:\n- name: t\n  auth: gh\n  kind: {type: http, method: GET, base_url: 'https://x.io', path_template: /}\n",
        )
        .unwrap();
    assert_eq!(b.tools[0].auth.as_deref(), Some("gh"));
    assert!(b.tools[0].input.enabled, "defaults apply");
    let e = parse_bundle(
            "version: 1\ntools:\n- name: t\n  kind: {type: http, method: GET, base_url: 'https://x.io', path_template: /, auth_ref: 00000000-0000-0000-0000-000000000000}\n",
        )
        .unwrap_err();
    assert!(e.has("tools[0].kind.auth_ref"));
}
