use super::*;

#[test]
fn seals_and_opens() {
    let c = Cipher::ephemeral();
    let a = c.seal("access-token-123");
    let b = c.seal("access-token-123");
    assert_ne!(a, b, "random nonce per value");
    assert!(!a.contains("access-token"));
    assert_eq!(c.open(&a).unwrap(), "access-token-123");
    assert!(
        Cipher::ephemeral().open(&a).is_err(),
        "another key can't open it"
    );
    assert!(c.open("zz").is_err());
    assert!(c.open(&a[..a.len() - 2]).is_err(), "tampering is caught");
}

#[test]
fn key_file_is_created_once_and_reused() {
    let dir = tempfile::tempdir().unwrap();
    let first = Cipher::load_or_create(dir.path()).unwrap();
    let sealed = first.seal("t");
    let key = std::fs::read_to_string(dir.path().join(KEY_FILE)).unwrap();
    assert_eq!(key.len(), 64);
    let again = Cipher::load_or_create(dir.path()).unwrap();
    assert_eq!(again.open(&sealed).unwrap(), "t");

    std::fs::write(dir.path().join(KEY_FILE), "not hex").unwrap();
    assert!(Cipher::load_or_create(dir.path()).is_err());
}

#[test]
fn hex_round_trips() {
    assert_eq!(
        from_hex(&to_hex(&[0, 1, 254, 255])).unwrap(),
        [0, 1, 254, 255]
    );
    assert!(from_hex("abc").is_none());
    assert!(from_hex("zz").is_none());
}
