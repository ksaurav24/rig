use std::{path::PathBuf, str::FromStr, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};

use crate::{crypto::Vault, deploy::Deployer, proxy::Proxy, runtime::Runtime};

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
}

pub async fn open_db(url: &str) -> Result<SqlitePool> {
    let opts = SqliteConnectOptions::from_str(url)?
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(5));
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
