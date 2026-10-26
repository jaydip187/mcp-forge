use super::*;

#[test]
fn loopback_hosts() {
    for ok in [
        "localhost",
        "localhost:7410",
        "127.0.0.1",
        "127.0.0.1:1",
        "[::1]:7410",
        "[::1]",
        "LOCALHOST",
    ] {
        assert!(is_loopback_host(ok), "{ok}");
    }
    for bad in [
        "evil.com",
        "evil.com:7410",
        "10.0.0.1",
        "localhost.evil.com",
        "[::2]:1",
        "",
    ] {
        assert!(!is_loopback_host(bad), "{bad}");
    }
}

#[test]
fn ct_eq() {
    assert!(constant_time_eq(b"abc", b"abc"));
    assert!(!constant_time_eq(b"abc", b"abd"));
    assert!(!constant_time_eq(b"abc", b"abcd"));
}
