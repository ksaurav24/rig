//! OS Keychain token storage for Rig Desktop.
//!
//! Uses Keychain (macOS), Credential Manager (Windows), and Secret Service /
//! libsecret (Linux) via `keyring`. Tokens never enter the webview or JS heap.
//!
//! If no OS secret service is running (e.g. headless Linux CI or minimal WM
//! without gnome-keyring/kwallet), falls back to a `0600` local file inside
//! `<config_dir>/rig-desktop/tokens.fallback.json` with a warning logged.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};

const SERVICE_NAME: &str = "sh.rig.desktop";

pub struct TokenStore {
    fallback_path: PathBuf,
}

impl Default for TokenStore {
    fn default() -> Self {
        Self::new(
            dirs::config_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("rig-desktop")
                .join("tokens.fallback.json"),
        )
    }
}

impl TokenStore {
    pub fn new(fallback_path: PathBuf) -> Self {
        Self { fallback_path }
    }

    fn account(server_id: &str) -> String {
        format!("server:{server_id}")
    }

    pub fn set(&self, server_id: &str, secret: &str) -> Result<()> {
        let account = Self::account(server_id);
        match keyring::Entry::new(SERVICE_NAME, &account).and_then(|e| e.set_password(secret)) {
            Ok(()) => {
                // Clean up any stale fallback entry if keychain is available.
                let _ = self.remove_fallback(server_id);
                Ok(())
            }
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    "OS keychain unavailable; storing device token in 0600 fallback file"
                );
                self.set_fallback(server_id, secret)
            }
        }
    }

    pub fn get(&self, server_id: &str) -> Result<Option<String>> {
        let account = Self::account(server_id);
        match keyring::Entry::new(SERVICE_NAME, &account).and_then(|e| e.get_password()) {
            Ok(secret) if !secret.is_empty() => return Ok(Some(secret)),
            Ok(_) | Err(keyring::Error::NoEntry) => {}
            Err(err) => {
                tracing::debug!(error = %err, "keychain lookup fell back to local store");
            }
        }
        self.get_fallback(server_id)
    }

    pub fn delete(&self, server_id: &str) -> Result<()> {
        let account = Self::account(server_id);
        if let Ok(entry) = keyring::Entry::new(SERVICE_NAME, &account) {
            let _ = entry.delete_credential();
        }
        let _ = self.remove_fallback(server_id);
        Ok(())
    }

    fn load_fallback(&self) -> HashMap<String, String> {
        let Ok(raw) = std::fs::read_to_string(&self.fallback_path) else {
            return HashMap::new();
        };
        serde_json::from_str(&raw).unwrap_or_default()
    }

    fn save_fallback(&self, map: &HashMap<String, String>) -> Result<()> {
        if let Some(parent) = self.fallback_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let tmp = self.fallback_path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string(map)?)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(&tmp, &self.fallback_path)?;
        Ok(())
    }

    fn set_fallback(&self, server_id: &str, secret: &str) -> Result<()> {
        let mut map = self.load_fallback();
        map.insert(server_id.to_owned(), secret.to_owned());
        self.save_fallback(&map)
    }

    fn get_fallback(&self, server_id: &str) -> Result<Option<String>> {
        Ok(self.load_fallback().remove(server_id))
    }

    fn remove_fallback(&self, server_id: &str) -> Result<()> {
        if !self.fallback_path.exists() {
            return Ok(());
        }
        let mut map = self.load_fallback();
        if map.remove(server_id).is_some() {
            self.save_fallback(&map)?;
        }
        Ok(())
    }
}
