use super::*;

#[test]
fn base64_matches_rfc4648() {
    for (i, o) in [
        ("", ""),
        ("f", "Zg=="),
        ("fo", "Zm8="),
        ("foo", "Zm9v"),
        ("foobar", "Zm9vYmFy"),
    ] {
        assert_eq!(base64_encode(i.as_bytes()), o);
    }
}

#[test]
fn redaction_covers_encoded_forms() {
    let s = redact_all("a=s3cr+t b=s3cr%2Bt", &["s3cr+t".into()]);
    assert_eq!(s, "a=[REDACTED] b=[REDACTED]");
}
