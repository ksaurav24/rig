//! Persistent multi-server registry for Rig Desktop.
//!
//! Stores non-secret server metadata, display order, pinned TLS certificate
//! fingerprints, and read-only cached snapshots in `<config_dir>/rig-desktop/servers.json`.
//! Bearer tokens NEVER touch this file — they live exclusively in the OS keychain.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rig_api::{App, Database, Project, ServerMeta, SystemInfo};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServerHealth {
    #[default]
    Connecting,
    Online,
    Deploying,
    Degraded,
    Offline,
}

/// Cached read-only snapshot shown immediately on server switch or when offline.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CachedServerSnapshot {
    pub system: Option<SystemInfo>,
    pub projects: Vec<Project>,
    pub apps: Vec<App>,
    pub databases: Vec<Database>,
    pub cached_at: Option<String>,
}

/// One paired Rig server entry in the left switcher rail.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerEntry {
    pub id: String,
    pub name: String,
    pub url: String,
    /// Optional accent color name or hex for the switcher badge.
    pub accent: String,
    /// User-confirmed SHA-256 leaf TLS cert fingerprint for `lan` / `--tls internal`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned_tls_sha256: Option<String>,
    /// User email that paired this device.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_email: Option<String>,
    /// Organization name on this server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org_name: Option<String>,
    /// Last known server version/mode/capabilities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<ServerMeta>,
    /// Last known read-only state for instant switch (< 100ms) and offline view.
    #[serde(default)]
    pub cache: CachedServerSnapshot,
    pub created_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RegistryData {
    pub active_server_id: Option<String>,
    pub servers: Vec<ServerEntry>,
}

pub struct ServerRegistry {
    path: PathBuf,
    data: RegistryData,
}

impl ServerRegistry {
    pub fn default_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("rig-desktop")
            .join("servers.json")
    }

    pub fn load(path: PathBuf) -> Result<Self> {
        let data = if path.exists() {
            let raw = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            serde_json::from_str(&raw).unwrap_or_default()
        } else {
            RegistryData::default()
        };
        Ok(Self { path, data })
    }

    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let tmp = tmp_path(&self.path);
        let json = serde_json::to_string_pretty(&self.data)?;
        std::fs::write(&tmp, json).with_context(|| format!("writing {}", tmp.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(&tmp, &self.path)
            .with_context(|| format!("saving {}", self.path.display()))?;
        Ok(())
    }

    pub fn data(&self) -> &RegistryData {
        &self.data
    }

    pub fn list(&self) -> &[ServerEntry] {
        &self.data.servers
    }

    pub fn active_id(&self) -> Option<&str> {
        self.data.active_server_id.as_deref()
    }

    pub fn get(&self, id: &str) -> Option<&ServerEntry> {
        self.data.servers.iter().find(|s| s.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut ServerEntry> {
        self.data.servers.iter_mut().find(|s| s.id == id)
    }

    pub fn add(&mut self, entry: ServerEntry) -> Result<ServerEntry> {
        let id = entry.id.clone();
        self.data.servers.retain(|s| s.id != id);
        self.data.servers.push(entry.clone());
        self.data.active_server_id = Some(id);
        self.save()?;
        Ok(entry)
    }

    pub fn remove(&mut self, id: &str) -> Result<bool> {
        let before = self.data.servers.len();
        self.data.servers.retain(|s| s.id != id);
        let removed = self.data.servers.len() != before;
        if self.data.active_server_id.as_deref() == Some(id) {
            self.data.active_server_id = self.data.servers.first().map(|s| s.id.clone());
        }
        if removed {
            self.save()?;
        }
        Ok(removed)
    }

    pub fn set_active(&mut self, id: &str) -> Result<bool> {
        if !self.data.servers.iter().any(|s| s.id == id) {
            return Ok(false);
        }
        self.data.active_server_id = Some(id.to_owned());
        self.save()?;
        Ok(true)
    }

    /// Reorders servers to match `ordered_ids`, preserving any unknown IDs at the end.
    pub fn reorder(&mut self, ordered_ids: &[String]) -> Result<()> {
        let mut next = Vec::with_capacity(self.data.servers.len());
        for id in ordered_ids {
            if let Some(pos) = self.data.servers.iter().position(|s| &s.id == id) {
                next.push(self.data.servers.remove(pos));
            }
        }
        next.append(&mut self.data.servers);
        self.data.servers = next;
        self.save()?;
        Ok(())
    }

    pub fn update_cache(&mut self, id: &str, cache: CachedServerSnapshot) -> Result<()> {
        if let Some(s) = self.get_mut(id) {
            s.cache = cache;
            self.save()?;
        }
        Ok(())
    }

    pub fn update_meta(&mut self, id: &str, meta: ServerMeta) -> Result<()> {
        if let Some(s) = self.get_mut(id) {
            s.meta = Some(meta);
            self.save()?;
        }
        Ok(())
    }
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut os = path.as_os_str().to_os_string();
    os.push(".tmp");
    PathBuf::from(os)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: &str, name: &str) -> ServerEntry {
        ServerEntry {
            id: id.into(),
            name: name.into(),
            url: format!("https://{name}.example.com"),
            accent: "emerald".into(),
            pinned_tls_sha256: None,
            user_email: Some("dev@example.com".into()),
            org_name: Some("Acme".into()),
            meta: None,
            cache: CachedServerSnapshot::default(),
            created_at: "2026-11-24T00:00:00Z".into(),
        }
    }

    #[test]
    fn registry_crud_reorder_and_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("servers.json");
        let mut reg = ServerRegistry::load(path.clone()).unwrap();
        reg.add(sample("s1", "prod")).unwrap();
        reg.add(sample("s2", "staging")).unwrap();
        reg.add(sample("s3", "homelab")).unwrap();
        assert_eq!(reg.active_id(), Some("s3"));

        reg.reorder(&["s3".into(), "s1".into(), "s2".into()]).unwrap();
        let reloaded = ServerRegistry::load(path).unwrap();
        let ids: Vec<&str> = reloaded.list().iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["s3", "s1", "s2"]);
    }
}
