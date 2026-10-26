use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};

/// A reference to a secret (`env:NAME` or `keyring:NAME`), never the secret itself.
///
/// Parsing rejects anything else, so raw secret values cannot reach the store.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(into = "String")]
pub struct SecretRef(String);

impl SecretRef {
    pub const SCHEMES: [&'static str; 2] = ["env", "keyring"];

    pub fn parse(s: &str) -> Result<Self, String> {
        let Some((scheme, name)) = s.split_once(':') else {
            return Err(
                "must be a reference like `env:NAME` or `keyring:NAME`, not a raw secret".into(),
            );
        };
        if !Self::SCHEMES.contains(&scheme) {
            return Err(format!(
                "unknown secret scheme `{scheme}`; use `env:` or `keyring:`"
            ));
        }
        if name.is_empty() || name.len() > 256 || name.chars().any(char::is_whitespace) {
            return Err("secret name must be 1–256 characters without whitespace".into());
        }
        Ok(Self(s.to_owned()))
    }

    pub fn scheme(&self) -> &str {
        self.0.split_once(':').map_or("", |(s, _)| s)
    }

    pub fn name(&self) -> &str {
        self.0.split_once(':').map_or("", |(_, n)| n)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Turns a [`SecretRef`] into the secret value at call time.
///
/// Implementations may block (e.g. OS keyring); callers run them off the async runtime.
/// Error messages must never contain the secret value.
pub trait SecretResolver: Send + Sync {
    fn resolve(&self, r: &SecretRef) -> Result<String, String>;
}

impl fmt::Display for SecretRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<SecretRef> for String {
    fn from(r: SecretRef) -> Self {
        r.0
    }
}

impl<'de> Deserialize<'de> for SecretRef {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        SecretRef::parse(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
#[path = "../tests/unit/secret.rs"]
mod tests;
