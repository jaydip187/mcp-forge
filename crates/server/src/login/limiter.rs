//! Backoff for failed sign-in attempts, kept in memory per key (a username
//! or a client address).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// First lockout after the free attempts; doubles with each further failure.
const BASE_LOCK: Duration = Duration::from_secs(30);
const MAX_LOCK: Duration = Duration::from_secs(15 * 60);
/// Failures older than this are forgotten.
const FORGET_AFTER: Duration = Duration::from_secs(60 * 60);
/// Bound on tracked keys; stale entries are dropped past this.
const MAX_KEYS: usize = 10_000;

struct Entry {
    failures: u32,
    last: Instant,
    locked_until: Option<Instant>,
}

pub struct Limiter {
    free_attempts: u32,
    entries: Mutex<HashMap<String, Entry>>,
}

impl Limiter {
    pub fn new(free_attempts: u32) -> Self {
        Self {
            free_attempts,
            entries: Mutex::default(),
        }
    }

    /// `Err(wait)` while `key` is locked out.
    pub fn check(&self, key: &str, now: Instant) -> Result<(), Duration> {
        let entries = self.entries.lock().expect("limiter lock");
        match entries.get(key).and_then(|e| e.locked_until) {
            Some(until) if until > now => Err(until - now),
            _ => Ok(()),
        }
    }

    pub fn fail(&self, key: &str, now: Instant) {
        let mut entries = self.entries.lock().expect("limiter lock");
        if entries.len() >= MAX_KEYS {
            entries.retain(|_, e| now.duration_since(e.last) < FORGET_AFTER);
        }
        let e = entries.entry(key.to_owned()).or_insert(Entry {
            failures: 0,
            last: now,
            locked_until: None,
        });
        if now.duration_since(e.last) >= FORGET_AFTER {
            e.failures = 0;
        }
        e.failures += 1;
        e.last = now;
        if let Some(over) = e.failures.checked_sub(self.free_attempts) {
            let lock = BASE_LOCK.saturating_mul(1 << over.min(16)).min(MAX_LOCK);
            e.locked_until = Some(now + lock);
        }
    }

    pub fn succeed(&self, key: &str) {
        self.entries.lock().expect("limiter lock").remove(key);
    }
}
