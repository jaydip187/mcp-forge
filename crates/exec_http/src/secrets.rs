use std::collections::HashMap;

use forge_core::{SecretRef, SecretResolver};

/// Keyring service name under which `keyring:NAME` secrets are stored.
pub const KEYRING_SERVICE: &str = "mcp-forge";

/// Resolves `env:NAME` from the process environment and `keyring:NAME` from
/// the OS keyring (service [`KEYRING_SERVICE`], user `NAME`).
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultSecretResolver;

impl SecretResolver for DefaultSecretResolver {
    fn resolve(&self, r: &SecretRef) -> Result<String, String> {
        match r.scheme() {
            "env" => std::env::var(r.name())
                .map_err(|_| format!("environment variable `{}` is not set", r.name())),
            "keyring" => keyring::Entry::new(KEYRING_SERVICE, r.name())
                .and_then(|e| e.get_password())
                .map_err(|e| {
                    format!(
                        "keyring entry `{}` (service `{KEYRING_SERVICE}`) is unavailable: {e}",
                        r.name()
                    )
                }),
            other => Err(format!("unsupported secret scheme `{other}`")),
        }
    }
}

/// Fixed in-memory secrets keyed by the full reference (`env:X`). For tests.
#[derive(Debug, Default, Clone)]
pub struct MapSecretResolver(pub HashMap<String, String>);

impl MapSecretResolver {
    pub fn with(mut self, reference: &str, value: &str) -> Self {
        self.0.insert(reference.to_owned(), value.to_owned());
        self
    }
}

impl SecretResolver for MapSecretResolver {
    fn resolve(&self, r: &SecretRef) -> Result<String, String> {
        self.0
            .get(r.as_str())
            .cloned()
            .ok_or_else(|| format!("secret `{r}` is not set"))
    }
}
