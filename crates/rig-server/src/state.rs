use std::{
    path::PathBuf,
    str::FromStr,
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use rig_api::{ServerMode, TailscaleInfo};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};

use crate::{
    crypto::Vault,
    deploy::Deployer,
    events::EventBus,
    proxy::Proxy,
    runtime::{EngineInfo, Runtime},
};

#[derive(Debug, Clone)]
pub struct SystemSnapshot {
    pub engine: Option<EngineInfo>,
    pub proxy_ready: bool,
    pub cpu_percent: f32,
    pub memory_used: u64,
    pub memory_total: u64,
    pub disk_used: u64,
    pub disk_total: u64,
    pub sampled_at: Instant,
}

#[derive(Debug, Clone)]
pub struct NetworkSnapshot {
    pub public_ip: Option<String>,
    pub lan_addresses: Vec<String>,
    pub tailscale: Option<TailscaleInfo>,
    pub sampled_at: Instant,
}

/// Process-local in-memory caches for expensive host, engine, proxy, and
/// network probes so frequent dashboard polling never blocks on live I/O.
#[derive(Default)]
pub struct RuntimeCaches {
    pub system: RwLock<Option<SystemSnapshot>>,
    pub system_refresh: tokio::sync::Mutex<()>,
    pub network: RwLock<Option<NetworkSnapshot>>,
    pub network_refresh: tokio::sync::Mutex<()>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub data_dir: PathBuf,
    /// Mark the session cookie `Secure`. On when the dashboard is served over HTTPS.
    pub secure_cookies: bool,
    /// GitHub API base, no trailing slash.
    pub github_api: String,
    /// GitHub web base, no trailing slash.
    pub github_web: String,
    /// Seconds between branch polls; 0 turns polling off.
    pub poll_interval: u64,
    /// Allow alert webhooks to connect to loopback/private/link-local addresses.
    pub allow_private_alerts: bool,
    /// Allow Git clone and poller to connect to loopback/private/link-local addresses.
    pub allow_private_git: bool,
    /// Serve the interactive Scalar API docs at `/api/docs` (authenticated).
    pub enable_api_docs: bool,
    /// `full` serves the embedded dashboard alongside the API; `headless` serves the API only.
    pub mode: ServerMode,
}

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub vault: Vault,
    pub config: Arc<Config>,
    pub runtime: Arc<dyn Runtime>,
    pub proxy: Arc<dyn Proxy>,
    pub deploys: Arc<Deployer>,
    /// Who may open the dashboard; see `access::guard`.
    pub access: crate::access::DashboardAccess,
    pub caches: Arc<RuntimeCaches>,
    /// Process-local org-scoped event bus for `GET /api/v1/events`.
    pub events: Arc<EventBus>,
}

pub async fn open_db(url: &str) -> Result<SqlitePool> {
    let opts = SqliteConnectOptions::from_str(url)?
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(5))
        .pragma("cache_size", "-16000")
        .pragma("mmap_size", "67108864");
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(opts)
        .await
        .with_context(|| format!("opening database {url}"))?;
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("running migrations")?;
    Ok(pool)
}

/// RFC 3339 UTC with millisecond precision. Fixed width, so stored
/// timestamps compare correctly as strings.
pub fn now() -> String {
    ts(chrono::Utc::now())
}

pub fn ts(t: chrono::DateTime<chrono::Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub fn new_id() -> String {
    uuid::Uuid::now_v7().to_string()
}
