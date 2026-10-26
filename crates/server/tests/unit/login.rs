use super::*;
use std::time::Duration as StdDuration;

#[test]
fn session_lifetime() {
    let t0 = Utc::now();
    assert!(session_live(t0, t0, t0 + Duration::days(6)));
    assert!(
        !session_live(t0, t0, t0 + Duration::days(7)),
        "idle too long"
    );
    let busy = t0 + Duration::days(29);
    assert!(session_live(t0, busy, busy));
    assert!(!session_live(t0, busy, t0 + Duration::days(30)), "too old");
}

#[test]
fn reads_session_cookie() {
    let mut h = HeaderMap::new();
    h.append(COOKIE, "theme=dark; mcp_forge_session=abc".parse().unwrap());
    h.append(COOKIE, "other=1".parse().unwrap());
    assert_eq!(cookie(&h, SESSION_COOKIE), Some("abc"));
    assert_eq!(cookie(&h, "other"), Some("1"));
    assert_eq!(cookie(&h, "missing"), None);
}

#[test]
fn tokens_are_random_and_hashed() {
    let (a, b) = (new_token(), new_token());
    assert_eq!(a.len(), 64);
    assert_ne!(a, b);
    assert_eq!(token_hash(&a), token_hash(&a));
    assert_ne!(token_hash(&a), a);
}

#[test]
fn setup_codes() {
    let code = new_setup_code();
    assert_eq!(code.len(), 14);
    assert_eq!(code.matches('-').count(), 2);
    assert_eq!(normalize_code(&code.to_lowercase().replace('-', " ")), code);

    let login = Login::default();
    *login.setup_code.lock().unwrap() = Some(code.clone());
    assert!(!login.take_setup_code("AAAA-AAAA-AAAA"));
    assert!(login.take_setup_code(&code.replace('-', "")));
    assert!(!login.take_setup_code(&code), "a code works once");
}

#[test]
fn limiter_backs_off_then_forgives() {
    let l = Limiter::new(3);
    let t = Instant::now();
    for _ in 0..2 {
        l.fail("user:a", t);
    }
    assert!(l.check("user:a", t).is_ok());
    l.fail("user:a", t);
    let wait = l.check("user:a", t).unwrap_err();
    assert_eq!(wait, StdDuration::from_secs(30));
    assert!(l.check("user:a", t + wait).is_ok(), "lock expires");
    l.fail("user:a", t + wait);
    assert_eq!(
        l.check("user:a", t + wait).unwrap_err(),
        StdDuration::from_secs(60),
        "doubles"
    );
    assert!(l.check("user:b", t).is_ok(), "per key");
    l.succeed("user:a");
    assert!(l.check("user:a", t + wait).is_ok());
}

#[test]
fn credential_rules() {
    assert!(check_username("alice.b-c_d@x").is_ok());
    assert!(check_username("").is_err());
    assert!(check_username("has space").is_err());
    assert!(check_username(&"a".repeat(65)).is_err());
    assert!(check_password("1234567").is_err());
    assert!(check_password("12345678").is_ok());
    assert!(check_password(&"x".repeat(257)).is_err());
}

#[test]
fn hashes_verify() {
    let phc = hash_password("correct horse");
    assert!(phc.starts_with("$argon2id$"));
    assert!(verify_password("correct horse", &phc));
    assert!(!verify_password("wrong horse", &phc));
    assert!(!verify_password("correct horse", "not a hash"));
}
