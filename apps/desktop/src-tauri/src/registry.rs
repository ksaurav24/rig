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

fn default_reminder_interval_days() -> u32 {
    7
}

/// Per-server periodic snapshot backup reminder configuration & state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupReminder {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_reminder_interval_days")]
    pub interval_days: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_snapshot_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_snapshot_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_snapshot_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snoozed_until: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_notified_at: Option<String>,
}

impl Default for BackupReminder {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_days: default_reminder_interval_days(),
            last_snapshot_at: None,
            last_snapshot_path: None,
            last_snapshot_bytes: None,
            snoozed_until: None,
            last_notified_at: None,
        }
    }
}

impl BackupReminder {
    /// Returns true if the periodic reminder is enabled, not currently snoozed,
    /// and either no snapshot has been downloaded yet or `interval_days` have elapsed.
    pub fn is_due(&self, now: chrono::DateTime<chrono::Utc>) -> bool {
        if !self.enabled {
            return false;
        }
        if let Some(snoozed) = self
            .snoozed_until
            .as_deref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            && snoozed.with_timezone(&chrono::Utc) > now
        {
            return false;
        }
        let Some(last) = self
            .last_snapshot_at
            .as_deref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        else {
            return true;
        };
        let days = self.interval_days.clamp(1, 365) as i64;
        now >= last.with_timezone(&chrono::Utc) + chrono::Duration::days(days)
    }

    /// Returns true if a native OS notification should be sent now (throttled to at most once every 12h).
    pub fn should_notify(&self, now: chrono::DateTime<chrono::Utc>) -> bool {
        if !self.is_due(now) {
            return false;
        }
        if let Some(last_n) = self
            .last_notified_at
            .as_deref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            && now < last_n.with_timezone(&chrono::Utc) + chrono::Duration::hours(12)
        {
            return false;
        }
        true
    }
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
    /// Periodic full-server snapshot backup reminder configuration & history.
    #[serde(default)]
    pub backup_reminder: BackupReminder,
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

    pub fn path(&self) -> &Path {
        &self.path
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

    pub fn update_server(
        &mut self,
        id: &str,
        name: Option<String>,
        accent: Option<String>,
    ) -> Result<Option<ServerEntry>> {
        let Some(s) = self.get_mut(id) else {
            return Ok(None);
        };
        if let Some(n) = name {
            let trimmed = n.trim();
            if !trimmed.is_empty() {
                s.name = trimmed.to_owned();
            }
        }
        if let Some(a) = accent {
            let trimmed = a.trim();
            if !trimmed.is_empty() {
                s.accent = trimmed.to_owned();
            }
        }
        let cloned = s.clone();
        self.save()?;
        Ok(Some(cloned))
    }

    pub fn update_backup_reminder(
        &mut self,
        id: &str,
        enabled: bool,
        interval_days: u32,
    ) -> Result<Option<BackupReminder>> {
        let Some(s) = self.get_mut(id) else {
            return Ok(None);
        };
        s.backup_reminder.enabled = enabled;
        s.backup_reminder.interval_days = interval_days.clamp(1, 365);
        if !enabled {
            s.backup_reminder.snoozed_until = None;
        }
        let reminder = s.backup_reminder.clone();
        self.save()?;
        Ok(Some(reminder))
    }

    pub fn record_snapshot_download(
        &mut self,
        id: &str,
        path: String,
        bytes: u64,
    ) -> Result<Option<BackupReminder>> {
        let Some(s) = self.get_mut(id) else {
            return Ok(None);
        };
        let ts = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        s.backup_reminder.last_snapshot_at = Some(ts);
        s.backup_reminder.last_snapshot_path = Some(path);
        s.backup_reminder.last_snapshot_bytes = Some(bytes);
        s.backup_reminder.snoozed_until = None;
        s.backup_reminder.last_notified_at = None;
        let reminder = s.backup_reminder.clone();
        self.save()?;
        Ok(Some(reminder))
    }

    pub fn snooze_backup_reminder(
        &mut self,
        id: &str,
        hours: u32,
    ) -> Result<Option<BackupReminder>> {
        let Some(s) = self.get_mut(id) else {
            return Ok(None);
        };
        let until = chrono::Utc::now() + chrono::Duration::hours(hours.clamp(1, 720) as i64);
        s.backup_reminder.snoozed_until =
            Some(until.to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
        let reminder = s.backup_reminder.clone();
        self.save()?;
        Ok(Some(reminder))
    }

    pub fn mark_reminder_notified(&mut self, id: &str) -> Result<()> {
        if let Some(s) = self.get_mut(id) {
            s.backup_reminder.last_notified_at =
                Some(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
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
            backup_reminder: BackupReminder::default(),
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

    #[test]
    fn backup_reminder_due_and_snooze_logic() {
        let now = chrono::Utc::now();
        let mut rem = BackupReminder::default();
        assert!(!rem.is_due(now));

        rem.enabled = true;
        rem.interval_days = 7;
        assert!(rem.is_due(now));
        assert!(rem.should_notify(now));

        // After snapshot just now, not due.
        rem.last_snapshot_at = Some(now.to_rfc3339());
        assert!(!rem.is_due(now));

        // 8 days ago -> due again.
        rem.last_snapshot_at = Some((now - chrono::Duration::days(8)).to_rfc3339());
        assert!(rem.is_due(now));

        // Snoozed for 24h -> not due.
        rem.snoozed_until = Some((now + chrono::Duration::hours(24)).to_rfc3339());
        assert!(!rem.is_due(now));

        // Snooze expired, but notified 2h ago -> due for UI banner, throttled for OS notification.
        rem.snoozed_until = None;
        rem.last_notified_at = Some((now - chrono::Duration::hours(2)).to_rfc3339());
        assert!(rem.is_due(now));
        assert!(!rem.should_notify(now));
    }
}
