//! Secrets at rest, password hashing and opaque token helpers.

use std::{
    fs,
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::Path,
};

use anyhow::{Context, Result, anyhow, bail};
use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash},
};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE_NO_PAD},
};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit},
};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 24;

/// Encrypts values with XChaCha20-Poly1305 under the instance master key.
#[derive(Clone)]
pub struct Vault {
    cipher: XChaCha20Poly1305,
}

impl Vault {
    /// Loads `master.key` from the data directory, creating it (mode 0600)
    /// on first start. Refuses a key file readable by group or others.
    pub fn open(data_dir: &Path) -> Result<Self> {
        let path = data_dir.join("master.key");
        let key = if path.exists() {
            let mode = fs::metadata(&path)?.permissions().mode() & 0o777;
            if mode & 0o077 != 0 {
                bail!(
                    "{} has mode {mode:o}; it must not be readable by group or others (chmod 600)",
                    path.display()
                );
            }
            let raw = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
            if raw.len() != KEY_LEN {
                bail!("{} is not a valid {KEY_LEN}-byte key", path.display());
            }
            raw
        } else {
            let mut key = vec![0u8; KEY_LEN];
            rand::fill(&mut key[..]);
            let mut f = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
                .with_context(|| format!("creating {}", path.display()))?;
            f.write_all(&key)?;
            f.sync_all()?;
            tracing::info!(path = %path.display(), "generated master key; back it up with your data");
            key
        };
        let cipher =
            XChaCha20Poly1305::new_from_slice(&key).map_err(|_| anyhow!("bad key length"))?;
        Ok(Self { cipher })
    }

    #[cfg(test)]
    pub fn ephemeral() -> Self {
        let mut key = [0u8; KEY_LEN];
        rand::fill(&mut key[..]);
        Self {
            cipher: XChaCha20Poly1305::new_from_slice(&key).unwrap(),
        }
    }

    /// Returns base64(nonce || ciphertext) with a fresh random nonce.
    pub fn encrypt(&self, plaintext: &str) -> Result<String> {
        let mut nonce = [0u8; NONCE_LEN];
        rand::fill(&mut nonce[..]);
        let ct = self
            .cipher
            .encrypt(&XNonce::from(nonce), plaintext.as_bytes())
            .map_err(|_| anyhow!("encryption failed"))?;
        let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ct);
        Ok(STANDARD_NO_PAD.encode(out))
    }

    pub fn decrypt(&self, encoded: &str) -> Result<String> {
        let raw = STANDARD_NO_PAD
            .decode(encoded)
            .context("secret is not valid base64")?;
        if raw.len() < NONCE_LEN {
            bail!("secret is truncated");
        }
        let (nonce, ct) = raw.split_at(NONCE_LEN);
        let nonce: [u8; NONCE_LEN] = nonce.try_into().expect("split at NONCE_LEN");
        let pt = self
            .cipher
            .decrypt(&XNonce::from(nonce), ct)
            .map_err(|_| anyhow!("secret could not be decrypted (wrong master key?)"))?;
        String::from_utf8(pt).context("secret is not UTF-8")
    }
}

/// A random URL-safe token carrying `bytes` bytes of entropy.
pub fn random_token(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::fill(&mut buf[..]);
    URL_SAFE_NO_PAD.encode(buf)
}

/// SHA-256 hex digest. Tokens are high-entropy, so a fast hash is the right
/// tool for looking them up; passwords use Argon2id instead.
pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub fn constant_time_eq(a: &str, b: &str) -> bool {
    a.as_bytes().ct_eq(b.as_bytes()).into()
}

/// Argon2id with a random salt, as a PHC string. CPU-heavy: call from
/// `spawn_blocking`.
pub fn hash_password(password: &str) -> Result<String> {
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|h| h.to_string())
        .map_err(|e| anyhow!("hashing password: {e}"))
}

pub fn verify_password(password: &str, phc: &str) -> bool {
    PasswordHash::new(phc)
        .map(|h| {
            Argon2::default()
                .verify_password(password.as_bytes(), &h)
                .is_ok()
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_round_trips_and_uses_fresh_nonces() {
        let v = Vault::ephemeral();
        let a = v.encrypt("postgres://u:p@h/db").unwrap();
        let b = v.encrypt("postgres://u:p@h/db").unwrap();
        assert_ne!(a, b, "nonce must differ per message");
        assert_eq!(v.decrypt(&a).unwrap(), "postgres://u:p@h/db");
    }

    #[test]
    fn vault_rejects_tampering_and_other_keys() {
        let v = Vault::ephemeral();
        let enc = v.encrypt("secret").unwrap();
        let mut raw = STANDARD_NO_PAD.decode(&enc).unwrap();
        *raw.last_mut().unwrap() ^= 1;
        assert!(v.decrypt(&STANDARD_NO_PAD.encode(raw)).is_err());
        assert!(Vault::ephemeral().decrypt(&enc).is_err());
        assert!(v.decrypt("AAAA").is_err());
    }

    #[test]
    fn key_file_is_private_and_reused() {
        let dir = tempfile::tempdir().unwrap();
        let v1 = Vault::open(dir.path()).unwrap();
        let path = dir.path().join("master.key");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let enc = v1.encrypt("x").unwrap();
        assert_eq!(Vault::open(dir.path()).unwrap().decrypt(&enc).unwrap(), "x");

        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(Vault::open(dir.path()).is_err());
    }

    #[test]
    fn passwords_verify() {
        let h = hash_password("correct horse battery").unwrap();
        assert!(h.starts_with("$argon2id$"));
        assert!(verify_password("correct horse battery", &h));
        assert!(!verify_password("wrong", &h));
        assert!(!verify_password("x", "not-a-hash"));
    }

    #[test]
    fn tokens_are_unique_and_hash_stably() {
        let a = random_token(32);
        assert_ne!(a, random_token(32));
        assert_eq!(hash_token(&a), hash_token(&a));
        assert_eq!(hash_token(&a).len(), 64);
        assert!(constant_time_eq("abc", "abc") && !constant_time_eq("abc", "abd"));
    }
}
