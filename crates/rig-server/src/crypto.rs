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
    aead::{Aead, KeyInit, Payload},
};
use hmac::{Hmac, KeyInit as HmacKeyInit, Mac};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 24;
pub const BACKUP_MAGIC: &[u8; 8] = b"RIGBK1\0\0";
const STREAM_BASE_LEN: usize = 16;
pub const BACKUP_CHUNK_SIZE: usize = 64 * 1024;

/// Encrypts values with XChaCha20-Poly1305 under the instance master key.
#[derive(Clone)]
pub struct Vault {
    cipher: XChaCha20Poly1305,
    hmac_key: [u8; KEY_LEN],
}

fn derive_hmac_key(master: &[u8]) -> [u8; KEY_LEN] {
    let mut h = Sha256::new();
    h.update(b"rig-hmac-subkey-v1:");
    h.update(master);
    h.finalize().into()
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
        let hmac_key = derive_hmac_key(&key);
        Ok(Self { cipher, hmac_key })
    }

    #[cfg(test)]
    pub fn ephemeral() -> Self {
        let mut key = [0u8; KEY_LEN];
        rand::fill(&mut key[..]);
        Self {
            cipher: XChaCha20Poly1305::new_from_slice(&key).unwrap(),
            hmac_key: derive_hmac_key(&key),
        }
    }

    /// Keyed HMAC-SHA256 hex digest using a subkey derived from `master.key`.
    /// Prevents offline brute-force of lower-entropy secrets if only `rig.db` leaks.
    pub fn hmac_sha256(&self, domain: &str, data: &[u8]) -> String {
        let mut mac = <Hmac<Sha256> as HmacKeyInit>::new_from_slice(&self.hmac_key)
            .expect("HMAC accepts 32-byte keys");
        mac.update(domain.as_bytes());
        mac.update(&[0]);
        mac.update(data);
        hex::encode(mac.finalize().into_bytes())
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

    /// Encrypt one chunk of a streaming backup file. `chunk_index` and `is_final`
    /// are bound into the nonce and AAD so chunks cannot be reordered or truncated.
    pub fn encrypt_chunk(
        &self,
        base: &[u8; STREAM_BASE_LEN],
        chunk_index: u64,
        is_final: bool,
        plaintext: &[u8],
    ) -> Result<Vec<u8>> {
        let mut nonce = [0u8; NONCE_LEN];
        nonce[..STREAM_BASE_LEN].copy_from_slice(base);
        nonce[STREAM_BASE_LEN..].copy_from_slice(&chunk_index.to_le_bytes());
        let mut aad = [0u8; 9];
        aad[..8].copy_from_slice(&chunk_index.to_le_bytes());
        aad[8] = u8::from(is_final);
        self.cipher
            .encrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: plaintext,
                    aad: &aad,
                },
            )
            .map_err(|_| anyhow!("backup chunk encryption failed"))
    }

    /// Decrypt one chunk of a streaming backup file.
    pub fn decrypt_chunk(
        &self,
        base: &[u8; STREAM_BASE_LEN],
        chunk_index: u64,
        is_final: bool,
        ciphertext: &[u8],
    ) -> Result<Vec<u8>> {
        let mut nonce = [0u8; NONCE_LEN];
        nonce[..STREAM_BASE_LEN].copy_from_slice(base);
        nonce[STREAM_BASE_LEN..].copy_from_slice(&chunk_index.to_le_bytes());
        let mut aad = [0u8; 9];
        aad[..8].copy_from_slice(&chunk_index.to_le_bytes());
        aad[8] = u8::from(is_final);
        self.cipher
            .decrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|_| anyhow!("backup file could not be decrypted (corrupt or wrong master key?)"))
    }
}

/// Encrypt any legacy plaintext `apps.webhook_secret` and `alert_channels.signing_secret`
/// rows at startup so secrets are never stored in plaintext in SQLite.
pub async fn migrate_plaintext_secrets(db: &sqlx::SqlitePool, vault: &Vault) -> Result<()> {
    let apps: Vec<(String, String)> =
        sqlx::query_as("SELECT id, webhook_secret FROM apps")
            .fetch_all(db)
            .await?;
    for (id, secret) in apps {
        if vault.decrypt(&secret).is_err() {
            let enc = vault.encrypt(&secret)?;
            sqlx::query("UPDATE apps SET webhook_secret = ? WHERE id = ?")
                .bind(enc)
                .bind(id)
                .execute(db)
                .await?;
        }
    }
    let channels: Vec<(String, String)> =
        sqlx::query_as("SELECT id, signing_secret FROM alert_channels")
            .fetch_all(db)
            .await?;
    for (id, secret) in channels {
        if vault.decrypt(&secret).is_err() {
            let enc = vault.encrypt(&secret)?;
            sqlx::query("UPDATE alert_channels SET signing_secret = ? WHERE id = ?")
                .bind(enc)
                .bind(id)
                .execute(db)
                .await?;
        }
    }
    Ok(())
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
