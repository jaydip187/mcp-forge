//! Encryption for tokens the store keeps (OAuth access and refresh tokens).
//!
//! AES-256-GCM with a random key in `secret.key` next to the database, so a
//! copy of the database alone doesn't leak tokens. Stored values are
//! `hex(nonce || ciphertext+tag)`.

use std::io::Write;
use std::path::Path;

use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
use ring::rand::{SecureRandom, SystemRandom};

use crate::StoreError;

pub(crate) const KEY_FILE: &str = "secret.key";

pub(crate) struct Cipher {
    key: LessSafeKey,
    rng: SystemRandom,
}

impl std::fmt::Debug for Cipher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Cipher { .. }")
    }
}

impl Cipher {
    fn from_bytes(bytes: &[u8; 32]) -> Self {
        let key = UnboundKey::new(&AES_256_GCM, bytes).expect("32-byte AES-256 key");
        Self {
            key: LessSafeKey::new(key),
            rng: SystemRandom::new(),
        }
    }

    /// A key that lives only as long as the process (in-memory stores).
    pub(crate) fn ephemeral() -> Self {
        let mut bytes = [0u8; 32];
        SystemRandom::new()
            .fill(&mut bytes)
            .expect("system randomness");
        Self::from_bytes(&bytes)
    }

    /// Reads the key from `dir/secret.key`, creating it (owner-only on Unix)
    /// on first use.
    pub(crate) fn load_or_create(dir: &Path) -> Result<Self, StoreError> {
        let path = dir.join(KEY_FILE);
        let io = |source| StoreError::DataDir {
            path: path.clone(),
            source,
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let bytes = from_hex(text.trim())
                    .and_then(|b| <[u8; 32]>::try_from(b).ok())
                    .ok_or_else(|| {
                        StoreError::Corrupt(format!("{} is not a 64-digit hex key", path.display()))
                    })?;
                Ok(Self::from_bytes(&bytes))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let mut bytes = [0u8; 32];
                SystemRandom::new()
                    .fill(&mut bytes)
                    .expect("system randomness");
                let mut options = std::fs::OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                let mut f = options.open(&path).map_err(io)?;
                f.write_all(to_hex(&bytes).as_bytes()).map_err(io)?;
                Ok(Self::from_bytes(&bytes))
            }
            Err(e) => Err(io(e)),
        }
    }

    pub(crate) fn seal(&self, plain: &str) -> String {
        let mut nonce = [0u8; NONCE_LEN];
        self.rng.fill(&mut nonce).expect("system randomness");
        let mut buf = plain.as_bytes().to_vec();
        self.key
            .seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::empty(), &mut buf)
            .expect("AES-GCM sealing can't fail for in-memory buffers");
        let mut out = nonce.to_vec();
        out.extend_from_slice(&buf);
        to_hex(&out)
    }

    pub(crate) fn open(&self, sealed: &str) -> Result<String, StoreError> {
        let corrupt =
            || StoreError::Corrupt("an encrypted token can't be read (wrong secret.key?)".into());
        let bytes = from_hex(sealed).ok_or_else(corrupt)?;
        if bytes.len() < NONCE_LEN {
            return Err(corrupt());
        }
        let (nonce, rest) = bytes.split_at(NONCE_LEN);
        let nonce = Nonce::try_assume_unique_for_key(nonce).map_err(|_| corrupt())?;
        let mut buf = rest.to_vec();
        let plain = self
            .key
            .open_in_place(nonce, Aad::empty(), &mut buf)
            .map_err(|_| corrupt())?;
        String::from_utf8(plain.to_vec()).map_err(|_| corrupt())
    }
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
#[path = "../tests/unit/cipher.rs"]
mod tests;
