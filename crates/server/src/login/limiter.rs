//! Backoff for failed sign-in attempts per key (a username or a client
//! address). The rules live here; the entries live in the store, so a
//! restart doesn't reset them.

use chrono::{DateTime, Duration, Utc};
use forge_store::Backoff;

/// First lockout after the free attempts; doubles with each further failure.
fn base_lock() -> Duration {
    Duration::seconds(30)
}

fn max_lock() -> Duration {
    Duration::minutes(15)
}

/// Failures older than this are forgotten.
pub fn forget_after() -> Duration {
    Duration::hours(1)
}

pub struct Limiter {
    free_attempts: u32,
}

impl Limiter {
    pub fn new(free_attempts: u32) -> Self {
        Self { free_attempts }
    }

    /// `Err(wait)` while `entry` is locked out at `now`.
    pub fn check(entry: Option<&Backoff>, now: DateTime<Utc>) -> Result<(), Duration> {
        match entry.and_then(|e| e.locked_until) {
            Some(until) if until > now => Err(until - now),
            _ => Ok(()),
        }
    }

    /// The entry for `key` after one more failure at `now`.
    pub fn fail(&self, entry: Option<Backoff>, key: &str, now: DateTime<Utc>) -> Backoff {
        let mut e = entry.unwrap_or_else(|| Backoff {
            key: key.to_owned(),
            failures: 0,
            last_failure_at: now,
            locked_until: None,
        });
        if now - e.last_failure_at >= forget_after() {
            e.failures = 0;
            e.locked_until = None;
        }
        e.failures = e.failures.saturating_add(1);
        e.last_failure_at = now;
        if let Some(over) = e.failures.checked_sub(self.free_attempts) {
            let lock = (base_lock() * (1i32 << over.min(16))).min(max_lock());
            e.locked_until = Some(now + lock);
        }
        e
    }
}
