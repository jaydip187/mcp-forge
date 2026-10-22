//! Password hashing (argon2id with the OWASP-recommended defaults) and the
//! rules for usernames and passwords.

use std::sync::OnceLock;

use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;

pub const MIN_PASSWORD_LEN: usize = 8;
/// Upper bound so a huge password can't make hashing expensive.
pub const MAX_PASSWORD_LEN: usize = 256;
pub const MAX_USERNAME_LEN: usize = 64;

/// A PHC string with a fresh random salt. CPU-heavy: call off the async runtime.
pub fn hash_password(password: &str) -> String {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .expect("argon2 with default parameters accepts any input")
        .to_string()
}

/// Whether `password` matches the PHC string. CPU-heavy, like hashing.
pub fn verify_password(password: &str, phc: &str) -> bool {
    PasswordHash::new(phc).is_ok_and(|h| {
        Argon2::default()
            .verify_password(password.as_bytes(), &h)
            .is_ok()
    })
}

/// Spends as long as a real check, so an unknown username can't be told
/// apart from a wrong password by timing.
pub fn verify_dummy(password: &str) {
    static DUMMY: OnceLock<String> = OnceLock::new();
    let phc = DUMMY.get_or_init(|| hash_password("dummy password for timing"));
    let _ = verify_password(password, phc);
}

pub fn check_username(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > MAX_USERNAME_LEN {
        return Err(format!("must be 1 to {MAX_USERNAME_LEN} characters"));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '@'))
    {
        return Err("may only contain letters, digits and . _ - @".into());
    }
    Ok(())
}

pub fn check_password(password: &str) -> Result<(), String> {
    let n = password.chars().count();
    if n < MIN_PASSWORD_LEN {
        return Err(format!("must be at least {MIN_PASSWORD_LEN} characters"));
    }
    if n > MAX_PASSWORD_LEN {
        return Err(format!("must be at most {MAX_PASSWORD_LEN} characters"));
    }
    Ok(())
}
