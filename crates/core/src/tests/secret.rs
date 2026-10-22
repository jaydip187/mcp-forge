use super::*;

#[test]
fn accepts_refs() {
    let r = SecretRef::parse("env:GITHUB_TOKEN").unwrap();
    assert_eq!(r.scheme(), "env");
    assert_eq!(r.name(), "GITHUB_TOKEN");
    assert_eq!(SecretRef::parse("keyring:gh").unwrap().scheme(), "keyring");
}

#[test]
fn rejects_raw_values() {
    for bad in ["ghp_abc123", "", "env:", "vault:x", "env:has space", ":x"] {
        assert!(SecretRef::parse(bad).is_err(), "{bad:?} should be rejected");
    }
}

#[test]
fn serde_round_trip_and_rejects_raw() {
    let r: SecretRef = serde_json::from_str("\"env:X\"").unwrap();
    assert_eq!(serde_json::to_string(&r).unwrap(), "\"env:X\"");
    assert!(serde_json::from_str::<SecretRef>("\"sk-live-123\"").is_err());
}
