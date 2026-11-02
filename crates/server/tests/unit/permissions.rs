use super::*;

#[test]
fn needs_by_route() {
    let pid = Uuid::new_v4();
    let p = |rest: &str| format!("/projects/{pid}{rest}");
    assert_eq!(need(&Method::GET, "/auth/status"), Need::SignedIn);
    assert_eq!(need(&Method::GET, "/info"), Need::SignedIn);
    assert_eq!(need(&Method::GET, "/projects"), Need::SignedIn);
    assert_eq!(need(&Method::POST, "/projects"), Need::Admin);
    assert_eq!(need(&Method::GET, &p("")), Need::Read(pid));
    assert_eq!(need(&Method::PUT, &p("")), Need::Write(pid), "rename");
    assert_eq!(need(&Method::DELETE, &p("")), Need::Admin);
    assert_eq!(need(&Method::GET, &p("/tools")), Need::Read(pid));
    assert_eq!(
        need(&Method::POST, &p("/tools/x/test")),
        Need::Write(pid),
        "test console"
    );
    assert_eq!(need(&Method::DELETE, &p("/api-keys/x")), Need::Write(pid));
    assert_eq!(
        need(&Method::POST, &p("/prompts/x/render")),
        Need::Read(pid),
        "prompt preview"
    );
    assert_eq!(need(&Method::POST, &p("/prompts")), Need::Write(pid));
    assert_eq!(
        need(&Method::POST, &p("/tools/check")),
        Need::Write(pid),
        "checks need what create needs"
    );
    assert_eq!(need(&Method::GET, &p("/calls")), Need::Read(pid));
    assert_eq!(
        need(&Method::GET, "/projects/not-a-uuid/tools"),
        Need::Admin
    );
    assert_eq!(need(&Method::GET, "/users"), Need::Admin);
    assert_eq!(need(&Method::PUT, "/users/x/access/y"), Need::Admin);
    assert_eq!(need(&Method::GET, "/approvals"), Need::SignedIn);
    assert_eq!(
        need(&Method::POST, "/oauth/requests/x/approve"),
        Need::SignedIn
    );
    assert_eq!(
        need(&Method::GET, "/something-new"),
        Need::Admin,
        "unknown routes are admin-only"
    );
}
