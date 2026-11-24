//! Tauri IPC command boundary for Rig Desktop (§5.1).
//!
//! The React webview NEVER performs direct HTTP requests or touches bearer tokens.
//! Every command validates inputs, delegates to `rig-client` or native Rust
//! subsystems (keychain, log ring buffer, `.env` diff, local Git, terminal WS),
//! and returns lean, typed DTOs.

use std::sync::Arc;
use std::time::Instant;

use rig_api::{
    AddDomainRequest, AddMemberRequest, AlertChannel, AlertTestResult, ApiToken, App, AuditEntry,
    Backup, CreateAlertChannelRequest, CreateAppRequest, CreateDatabaseRequest,
    CreatePairingRequest, CreateProjectRequest, CreatedAlertChannel, Database, DatabaseCredentials,
    DatabaseEngineInfo, Deployment, DeviceLoginRequest, Domain, EnvVar, ExchangePairingRequest,
    GithubStatus, LogLine, Me, Member, MemberWithPassword, NetworkInfo, Organization, PairingCode,
    Project, RevealedEnvVar, ServerEvent, ServerMeta, SetEnvVarRequest, SetupRequest, SetupStatus,
    SystemInfo, UpdateAppRequest, UpdateMemberRequest, UpdateOrganizationRequest,
    UpdateProjectRequest,
};
use rig_client::{AppLogQuery, Client, ClientError, ClientOptions, MIN_API_VERSION, TlsProbe};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State, ipc::Channel};

use crate::connection::ConnectionManager;
use crate::env_diff::{EnvDiffItem, EnvDiffPreview, diff_env, to_bulk_request};
use crate::keychain::TokenStore;
use crate::local_git::{
    DiscoveredRepo, LocalGitStatus, deploy_local_checkout, discover_repos, inspect_repo,
};
use crate::log_engine::{
    FRAME_FLUSH_INTERVAL, LogBufferStats, LogChannelFrame, LogEngine, LogQueryResult,
    MAX_LINES_PER_BATCH,
};
use crate::registry::{CachedServerSnapshot, ServerEntry, ServerHealth, ServerRegistry};
use crate::terminal::{TerminalFrame, TerminalManager};
use tokio::sync::RwLock;

pub struct DesktopState {
    pub started_at: Instant,
    pub registry: Arc<RwLock<ServerRegistry>>,
    pub tokens: Arc<TokenStore>,
    pub connections: Arc<ConnectionManager>,
    pub logs: Arc<LogEngine>,
    pub terminals: Arc<TerminalManager>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerView {
    pub id: String,
    pub name: String,
    pub url: String,
    pub accent: String,
    pub pinned_tls_sha256: Option<String>,
    pub user_email: Option<String>,
    pub org_name: Option<String>,
    pub meta: Option<ServerMeta>,
    pub health: ServerHealth,
    pub health_error: Option<String>,
    pub cache: CachedServerSnapshot,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServersStatePayload {
    pub active_server_id: Option<String>,
    pub servers: Vec<ServerView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeServerResult {
    pub normalized_url: String,
    pub tls: TlsProbe,
    pub meta: Option<ServerMeta>,
    pub setup: Option<SetupStatus>,
    pub needs_tls_pin_confirmation: bool,
    pub api_compatible: bool,
    pub compatibility_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerSnapshot {
    pub server_id: String,
    pub from_cache: bool,
    pub system: Option<SystemInfo>,
    pub projects: Vec<Project>,
    pub apps: Vec<App>,
    pub databases: Vec<Database>,
    pub recent_events: Vec<ServerEvent>,
    pub meta: Option<ServerMeta>,
    pub fetch_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppDetailSnapshot {
    pub app: App,
    pub deployments: Vec<Deployment>,
    pub env: Vec<EnvVar>,
    pub domains: Vec<Domain>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerfDiagnostics {
    pub uptime_ms: u64,
    pub server_count: usize,
    pub log_buffers: Vec<LogBufferStats>,
    pub client_version: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ServerSettingsSnapshot {
    pub me: Option<Me>,
    pub network: Option<NetworkInfo>,
    pub devices: Vec<ApiToken>,
    pub tokens: Vec<ApiToken>,
    pub members: Vec<Member>,
    pub alerts: Vec<AlertChannel>,
    pub audit: Vec<AuditEntry>,
    pub github: Option<GithubStatus>,
}

fn default_device_name() -> String {
    let host = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "desktop".into());
    format!("{host} · Rig Desktop")
}

fn normalize_server_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("Enter a server URL (e.g. https://rig.example.com)".into());
    }
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_owned()
    } else if trimmed.starts_with("127.0.0.1") || trimmed.starts_with("localhost") {
        format!("http://{trimmed}")
    } else {
        format!("https://{trimmed}")
    };
    let parsed = url::Url::parse(&with_scheme).map_err(|e| format!("Invalid URL: {e}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("Only http:// and https:// URLs are supported".into());
    }
    Ok(parsed.as_str().trim_end_matches('/').to_owned())
}

fn pick_accent(index: usize) -> &'static str {
    const PALETTE: &[&str] = &[
        "emerald", "sky", "violet", "amber", "rose", "cyan", "indigo", "teal",
    ];
    PALETTE[index % PALETTE.len()]
}

async fn client_for_server(
    state: &DesktopState,
    server_id: &str,
) -> Result<(ServerEntry, Client), String> {
    let entry = state
        .registry
        .read()
        .await
        .get(server_id)
        .cloned()
        .ok_or_else(|| "Server not found".to_string())?;
    let client = ConnectionManager::build_client(&entry, &state.tokens)?;
    Ok((entry, client))
}

// ------------------------------------------------------------ server registry

#[tauri::command]
pub async fn list_servers(state: State<'_, DesktopState>) -> Result<ServersStatePayload, String> {
    let reg = state.registry.read().await;
    let health = state.connections.health_map().await;
    let servers = reg
        .list()
        .iter()
        .map(|s| {
            let (h, err) = health
                .get(&s.id)
                .cloned()
                .unwrap_or((ServerHealth::Connecting, None));
            ServerView {
                id: s.id.clone(),
                name: s.name.clone(),
                url: s.url.clone(),
                accent: s.accent.clone(),
                pinned_tls_sha256: s.pinned_tls_sha256.clone(),
                user_email: s.user_email.clone(),
                org_name: s.org_name.clone(),
                meta: s.meta.clone(),
                health: h,
                health_error: err,
                cache: s.cache.clone(),
                created_at: s.created_at.clone(),
            }
        })
        .collect();
    Ok(ServersStatePayload {
        active_server_id: reg.active_id().map(str::to_owned),
        servers,
    })
}

/// Probes a server's TLS certificate, `/api/v1/meta`, and `/api/v1/setup` before pairing.
#[tauri::command]
pub async fn probe_server(
    url: String,
    accept_fingerprint: Option<String>,
) -> Result<ProbeServerResult, String> {
    let normalized_url = normalize_server_url(&url)?;
    let tls = rig_client::probe_tls(&normalized_url)
        .await
        .map_err(|e| e.to_string())?;

    let needs_pin = tls.is_https && !tls.trusted_by_roots;
    let pin_to_use = if needs_pin {
        match accept_fingerprint.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(fp) => Some(fp.to_owned()),
            None => {
                // Return early so the user explicitly confirms the SHA-256 fingerprint (TOFU).
                return Ok(ProbeServerResult {
                    normalized_url,
                    tls,
                    meta: None,
                    setup: None,
                    needs_tls_pin_confirmation: true,
                    api_compatible: true,
                    compatibility_note: None,
                });
            }
        }
    } else {
        None
    };

    let client = Client::build(ClientOptions {
        base_url: normalized_url.clone(),
        token: None,
        pinned_tls_sha256: pin_to_use,
        user_agent: concat!("rig-desktop/", env!("CARGO_PKG_VERSION")).into(),
        verbose: false,
        warn_skew: false,
    })
    .map_err(|e| e.to_string())?;

    let meta = client.meta().await.ok();
    let setup = client.setup_status().await.ok();

    let (api_compatible, compatibility_note) = match &meta {
        Some(m) if m.api_version < MIN_API_VERSION => (
            false,
            Some(format!(
                "Server API v{} is older than the minimum supported API v{MIN_API_VERSION}. Upgrade Rig on this server.",
                m.api_version
            )),
        ),
        None => (
            true,
            Some("Server did not expose /api/v1/meta (older Rig version). Some desktop features may be unavailable.".into()),
        ),
        _ => (true, None),
    };

    Ok(ProbeServerResult {
        normalized_url,
        tls,
        meta,
        setup,
        needs_tls_pin_confirmation: false,
        api_compatible,
        compatibility_note,
    })
}

/// Initializes a brand-new (or headless) Rig server and pairs this desktop app automatically.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn init_server_setup(
    app: AppHandle,
    state: State<'_, DesktopState>,
    url: String,
    pinned_tls_sha256: Option<String>,
    server_name: Option<String>,
    setup_token: String,
    org_name: String,
    name: String,
    email: String,
    password: String,
) -> Result<ServerView, String> {
    let normalized_url = normalize_server_url(&url)?;
    let client = Client::build(ClientOptions {
        base_url: normalized_url.clone(),
        token: None,
        pinned_tls_sha256: pinned_tls_sha256.clone(),
        user_agent: concat!("rig-desktop/", env!("CARGO_PKG_VERSION")).into(),
        verbose: false,
        warn_skew: false,
    })
    .map_err(|e| e.to_string())?;

    client
        .complete_setup(&SetupRequest {
            token: setup_token.trim().to_owned(),
            name: name.trim().to_owned(),
            email: email.trim().to_owned(),
            password: password.clone(),
            org_name: org_name.trim().to_owned(),
        })
        .await
        .map_err(|e| e.to_string())?;

    let paired = client
        .device_login(&DeviceLoginRequest {
            email: email.trim().to_owned(),
            password,
            code: None,
            challenge_token: None,
            device_name: default_device_name(),
        })
        .await
        .map_err(|e| e.to_string())?;

    finish_add_server(app, &state, normalized_url, pinned_tls_sha256, server_name, paired).await
}

/// Exchanges a one-time pairing code (`rig devices pair`) for a device token and saves the server.
#[tauri::command]
pub async fn pair_server(
    app: AppHandle,
    state: State<'_, DesktopState>,
    url: String,
    code: String,
    server_name: Option<String>,
    pinned_tls_sha256: Option<String>,
) -> Result<ServerView, String> {
    let normalized_url = normalize_server_url(&url)?;
    let client = Client::build(ClientOptions {
        base_url: normalized_url.clone(),
        token: None,
        pinned_tls_sha256: pinned_tls_sha256.clone(),
        user_agent: concat!("rig-desktop/", env!("CARGO_PKG_VERSION")).into(),
        verbose: false,
        warn_skew: false,
    })
    .map_err(|e| e.to_string())?;

    let paired = client
        .exchange_pairing(&ExchangePairingRequest {
            code: code.trim().to_owned(),
            device_name: default_device_name(),
        })
        .await
        .map_err(|e| e.to_string())?;

    finish_add_server(app, &state, normalized_url, pinned_tls_sha256, server_name, paired).await
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DeviceLoginResult {
    Paired {
        server: ServerView,
    },
    SecondFactorRequired {
        challenge_token: Option<String>,
        message: String,
    },
}

/// Password (+ optional TOTP) sign-in that mints a device token for Rig Desktop.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn login_server(
    app: AppHandle,
    state: State<'_, DesktopState>,
    url: String,
    email: String,
    password: String,
    totp_code: Option<String>,
    challenge_token: Option<String>,
    server_name: Option<String>,
    pinned_tls_sha256: Option<String>,
) -> Result<DeviceLoginResult, String> {
    let normalized_url = normalize_server_url(&url)?;
    let client = Client::build(ClientOptions {
        base_url: normalized_url.clone(),
        token: None,
        pinned_tls_sha256: pinned_tls_sha256.clone(),
        user_agent: concat!("rig-desktop/", env!("CARGO_PKG_VERSION")).into(),
        verbose: false,
        warn_skew: false,
    })
    .map_err(|e| e.to_string())?;

    match client
        .device_login(&DeviceLoginRequest {
            email: email.trim().to_owned(),
            password,
            code: totp_code.filter(|c| !c.trim().is_empty()),
            challenge_token: challenge_token.filter(|c| !c.trim().is_empty()),
            device_name: default_device_name(),
        })
        .await
    {
        Ok(paired) => {
            let server = finish_add_server(
                app,
                &state,
                normalized_url,
                pinned_tls_sha256,
                server_name,
                paired,
            )
            .await?;
            Ok(DeviceLoginResult::Paired { server })
        }
        Err(ClientError::SecondFactorRequired {
            message,
            challenge_token,
        }) => Ok(DeviceLoginResult::SecondFactorRequired {
            challenge_token,
            message,
        }),
        Err(err) => Err(err.to_string()),
    }
}

async fn finish_add_server(
    app: AppHandle,
    state: &DesktopState,
    normalized_url: String,
    pinned_tls_sha256: Option<String>,
    server_name: Option<String>,
    paired: rig_api::PairedDevice,
) -> Result<ServerView, String> {
    let id = uuid::Uuid::now_v7().to_string();
    state
        .tokens
        .set(&id, &paired.secret)
        .map_err(|e| format!("saving device token to keychain: {e}"))?;

    let display_name = server_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            if !paired.org.name.is_empty() {
                paired.org.name.clone()
            } else {
                url::Url::parse(&normalized_url)
                    .ok()
                    .and_then(|u| u.host_str().map(str::to_owned))
                    .unwrap_or_else(|| "Rig Server".into())
            }
        });

    let count = state.registry.read().await.list().len();
    let accent = if !paired.org.accent_color.is_empty() {
        paired.org.accent_color.clone()
    } else {
        pick_accent(count).to_owned()
    };

    let entry = ServerEntry {
        id: id.clone(),
        name: display_name,
        url: normalized_url,
        accent,
        pinned_tls_sha256,
        user_email: Some(paired.user.email),
        org_name: Some(paired.org.name),
        meta: None,
        cache: CachedServerSnapshot::default(),
        created_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
    };

    state
        .registry
        .write()
        .await
        .add(entry.clone())
        .map_err(|e| e.to_string())?;

    // Start the persistent lightweight /api/v1/events watcher for this server.
    state
        .connections
        .ensure_watcher(
            app,
            entry.clone(),
            Arc::clone(&state.tokens),
            Arc::clone(&state.registry),
        )
        .await;

    Ok(ServerView {
        id: entry.id,
        name: entry.name,
        url: entry.url,
        accent: entry.accent,
        pinned_tls_sha256: entry.pinned_tls_sha256,
        user_email: entry.user_email,
        org_name: entry.org_name,
        meta: entry.meta,
        health: ServerHealth::Connecting,
        health_error: None,
        cache: entry.cache,
        created_at: entry.created_at,
    })
}

/// Switches the active server: stops view-level streams for the old server,
/// keeps its cache intact, and returns instantly (< 10ms).
#[tauri::command]
pub async fn select_server(
    state: State<'_, DesktopState>,
    server_id: String,
) -> Result<bool, String> {
    state.logs.stop_all();
    state.terminals.close_all().await;
    state
        .registry
        .write()
        .await
        .set_active(&server_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn remove_server(
    state: State<'_, DesktopState>,
    server_id: String,
) -> Result<bool, String> {
    state.connections.stop_watcher(&server_id).await;
    state.logs.stop_all();
    state.terminals.close_all().await;
    let _ = state.tokens.delete(&server_id);
    state
        .registry
        .write()
        .await
        .remove(&server_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn reorder_servers(
    state: State<'_, DesktopState>,
    ordered_ids: Vec<String>,
) -> Result<(), String> {
    state
        .registry
        .write()
        .await
        .reorder(&ordered_ids)
        .map_err(|e| e.to_string())
}

// ------------------------------------------------------------ workspace data

/// Fetches live overview data in parallel; falls back cleanly to the cached
/// read-only snapshot if the server is currently offline (§5.4).
#[tauri::command]
pub async fn get_server_snapshot(
    state: State<'_, DesktopState>,
    server_id: String,
) -> Result<ServerSnapshot, String> {
    let started = Instant::now();
    let recent_events = state.connections.recent_events_for(&server_id).await;
    let (entry, client) = client_for_server(&state, &server_id).await?;

    let (sys_res, proj_res, apps_res, dbs_res, meta_res) = tokio::join!(
        client.system(),
        client.list_projects(),
        client.list_apps(None),
        client.list_databases(),
        client.meta(),
    );

    if let (Ok(sys), Ok(projects), Ok(apps), Ok(databases)) = (sys_res, proj_res, apps_res, dbs_res)
    {
        let meta = meta_res.ok().or(entry.meta.clone());
        let cache = CachedServerSnapshot {
            system: Some(sys.clone()),
            projects: projects.clone(),
            apps: apps.clone(),
            databases: databases.clone(),
            cached_at: Some(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
        };
        let mut reg = state.registry.write().await;
        let _ = reg.update_cache(&server_id, cache);
        if let Some(m) = &meta {
            let _ = reg.update_meta(&server_id, m.clone());
        }
        return Ok(ServerSnapshot {
            server_id,
            from_cache: false,
            system: Some(sys),
            projects,
            apps,
            databases,
            recent_events,
            meta,
            fetch_ms: started.elapsed().as_millis() as u64,
        });
    }

    // Offline fallback: return cached snapshot read-only.
    Ok(ServerSnapshot {
        server_id,
        from_cache: true,
        system: entry.cache.system,
        projects: entry.cache.projects,
        apps: entry.cache.apps,
        databases: entry.cache.databases,
        recent_events,
        meta: entry.meta,
        fetch_ms: started.elapsed().as_millis() as u64,
    })
}

#[tauri::command]
pub async fn get_app_detail(
    state: State<'_, DesktopState>,
    server_id: String,
    app_id: String,
) -> Result<AppDetailSnapshot, String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    let (app, deployments, env, domains) = tokio::try_join!(
        client.get_app(&app_id),
        client.list_deployments(&app_id),
        client.list_env(&app_id),
        client.list_domains(&app_id),
    )
    .map_err(|e| e.to_string())?;
    Ok(AppDetailSnapshot {
        app,
        deployments,
        env,
        domains,
    })
}

#[tauri::command]
pub async fn create_project(
    state: State<'_, DesktopState>,
    server_id: String,
    name: String,
    description: Option<String>,
) -> Result<Project, String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    client
        .create_project(&CreateProjectRequest {
            name,
            description: description.unwrap_or_default(),
        })
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_project(
    state: State<'_, DesktopState>,
    server_id: String,
    project_id: String,
    name: Option<String>,
    description: Option<String>,
) -> Result<Project, String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    client
        .update_project(&project_id, &UpdateProjectRequest { name, description })
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_project(
    state: State<'_, DesktopState>,
    server_id: String,
    project_id: String,
) -> Result<(), String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    client
        .delete_project(&project_id, Some("delete"), None)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_app(
    state: State<'_, DesktopState>,
    server_id: String,
    req: CreateAppRequest,
) -> Result<App, String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    client.create_app(&req).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_app(
    state: State<'_, DesktopState>,
    server_id: String,
    app_id: String,
    req: UpdateAppRequest,
) -> Result<App, String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    client
        .update_app(&app_id, &req)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_app(
    state: State<'_, DesktopState>,
    server_id: String,
    app_id: String,
) -> Result<(), String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    client.delete_app(&app_id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn app_action(
    state: State<'_, DesktopState>,
    server_id: String,
    app_id: String,
    action: String,
) -> Result<serde_json::Value, String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    match action.as_str() {
        "deploy" => {
            let d = client.deploy_app(&app_id).await.map_err(|e| e.to_string())?;
            serde_json::to_value(d).map_err(|e| e.to_string())
        }
        "start" => {
            let d = client.start_app(&app_id).await.map_err(|e| e.to_string())?;
            serde_json::to_value(d).map_err(|e| e.to_string())
        }
        "restart" => {
            let d = client.restart_app(&app_id).await.map_err(|e| e.to_string())?;
            serde_json::to_value(d).map_err(|e| e.to_string())
        }
        "stop" => {
            let a = client.stop_app(&app_id).await.map_err(|e| e.to_string())?;
            serde_json::to_value(a).map_err(|e| e.to_string())
        }
        other => Err(format!("unknown app action: {other}")),
    }
}

#[tauri::command]
pub async fn rollback_deployment(
    state: State<'_, DesktopState>,
    server_id: String,
    deployment_id: String,
) -> Result<Deployment, String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    client
        .rollback_deployment(&deployment_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn cancel_deployment(
    state: State<'_, DesktopState>,
    server_id: String,
    deployment_id: String,
) -> Result<Deployment, String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    client
        .cancel_deployment(&deployment_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn add_domain(
    state: State<'_, DesktopState>,
    server_id: String,
    app_id: String,
    req: AddDomainRequest,
) -> Result<Domain, String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    client
        .add_domain(&app_id, &req)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_domain(
    state: State<'_, DesktopState>,
    server_id: String,
    app_id: String,
    domain_id: String,
) -> Result<(), String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    client
        .delete_domain(&app_id, &domain_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_database_engines(
    state: State<'_, DesktopState>,
    server_id: String,
) -> Result<Vec<DatabaseEngineInfo>, String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    client
        .list_database_engines()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_database(
    state: State<'_, DesktopState>,
    server_id: String,
    req: CreateDatabaseRequest,
) -> Result<Database, String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    client
        .create_database(&req)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_database(
    state: State<'_, DesktopState>,
    server_id: String,
    db_id: String,
) -> Result<(), String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    client
        .delete_database(&db_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn database_action(
    state: State<'_, DesktopState>,
    server_id: String,
    db_id: String,
    action: String,
) -> Result<Database, String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    match action.as_str() {
        "start" => client.start_database(&db_id).await.map_err(|e| e.to_string()),
        "stop" => client.stop_database(&db_id).await.map_err(|e| e.to_string()),
        other => Err(format!("unknown database action: {other}")),
    }
}

#[tauri::command]
pub async fn reveal_database(
    state: State<'_, DesktopState>,
    server_id: String,
    db_id: String,
) -> Result<DatabaseCredentials, String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    client
        .reveal_database(&db_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_backups(
    state: State<'_, DesktopState>,
    server_id: String,
    db_id: String,
) -> Result<Vec<Backup>, String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    client.list_backups(&db_id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_backup(
    state: State<'_, DesktopState>,
    server_id: String,
    db_id: String,
) -> Result<Backup, String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    client
        .create_backup(&db_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn restore_backup(
    state: State<'_, DesktopState>,
    server_id: String,
    backup_id: String,
) -> Result<(), String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    client
        .restore_backup(&backup_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_backup(
    state: State<'_, DesktopState>,
    server_id: String,
    backup_id: String,
) -> Result<(), String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    client
        .delete_backup(&backup_id)
        .await
        .map_err(|e| e.to_string())
}

// -------------------------------------------------- full server settings CRUD

#[tauri::command]
pub async fn get_server_settings(
    state: State<'_, DesktopState>,
    server_id: String,
) -> Result<ServerSettingsSnapshot, String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    let (me, network, devices, tokens, members, alerts, audit, github) = tokio::join!(
        client.me(),
        client.network(),
        client.list_devices(),
        client.list_tokens(),
        client.list_members(),
        client.list_alert_channels(),
        client.list_audit(),
        client.github_status(),
    );
    Ok(ServerSettingsSnapshot {
        me: me.ok(),
        network: network.ok(),
        devices: devices.unwrap_or_default(),
        tokens: tokens.unwrap_or_default(),
        members: members.unwrap_or_default(),
        alerts: alerts.unwrap_or_default(),
        audit: audit.unwrap_or_default(),
        github: github.ok(),
    })
}

#[tauri::command]
pub async fn update_organization(
    state: State<'_, DesktopState>,
    server_id: String,
    req: UpdateOrganizationRequest,
) -> Result<Organization, String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    client
        .update_organization(&req)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn add_member(
    state: State<'_, DesktopState>,
    server_id: String,
    req: AddMemberRequest,
) -> Result<MemberWithPassword, String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    client.add_member(&req).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_member(
    state: State<'_, DesktopState>,
    server_id: String,
    user_id: String,
    req: UpdateMemberRequest,
) -> Result<Member, String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    client
        .update_member(&user_id, &req)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn remove_member(
    state: State<'_, DesktopState>,
    server_id: String,
    user_id: String,
) -> Result<(), String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    client
        .remove_member(&user_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn reset_member_password(
    state: State<'_, DesktopState>,
    server_id: String,
    user_id: String,
) -> Result<MemberWithPassword, String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    client
        .reset_member_password(&user_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_alert_channel(
    state: State<'_, DesktopState>,
    server_id: String,
    req: CreateAlertChannelRequest,
) -> Result<CreatedAlertChannel, String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    client
        .create_alert_channel(&req)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn test_alert_channel(
    state: State<'_, DesktopState>,
    server_id: String,
    id: String,
) -> Result<AlertTestResult, String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    client
        .test_alert_channel(&id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_alert_channel(
    state: State<'_, DesktopState>,
    server_id: String,
    id: String,
) -> Result<(), String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    client
        .delete_alert_channel(&id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn revoke_token(
    state: State<'_, DesktopState>,
    server_id: String,
    token_id: String,
) -> Result<(), String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    client
        .delete_token(&token_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_devices(
    state: State<'_, DesktopState>,
    server_id: String,
) -> Result<Vec<ApiToken>, String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    client.list_devices().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_pairing_code(
    state: State<'_, DesktopState>,
    server_id: String,
) -> Result<PairingCode, String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    client
        .create_pairing(&CreatePairingRequest::default())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn revoke_device(
    state: State<'_, DesktopState>,
    server_id: String,
    device_id: String,
) -> Result<(), String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    client
        .revoke_device(&device_id)
        .await
        .map_err(|e| e.to_string())
}

// -------------------------------------------------- high-perf Rust log engine

#[tauri::command]
pub async fn start_app_logs(
    state: State<'_, DesktopState>,
    server_id: String,
    app_id: String,
    stream_id: String,
    tail: Option<u32>,
    on_frame: Channel<LogChannelFrame>,
) -> Result<(), String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    let engine = Arc::clone(&state.logs);
    let stop_rx = engine.register_stream(&stream_id);

    tokio::spawn(async move {
        let _ = on_frame.send(LogChannelFrame::Status {
            stream_id: stream_id.clone(),
            state: "connecting".into(),
            message: None,
        });

        let (line_tx, mut line_rx) = tokio::sync::mpsc::channel::<LogLine>(4096);

        // Batch flusher: caps UI updates to <= 1 batch per 16ms animation frame.
        let flusher_engine = Arc::clone(&engine);
        let flusher_sid = stream_id.clone();
        let flusher_chan = on_frame.clone();
        let flusher = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(FRAME_FLUSH_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut batch = Vec::with_capacity(256);
            let mut last_buf = 0usize;
            let mut last_drop = 0u64;
            let mut seq_counter = 0u64;

            loop {
                tokio::select! {
                    maybe = line_rx.recv() => {
                        match maybe {
                            Some(ll) => {
                                seq_counter += 1;
                                let seq = if ll.seq > 0 { ll.seq as u64 } else { seq_counter };
                                let (parsed, buf, dropped) = flusher_engine.push_line(
                                    &flusher_sid,
                                    seq,
                                    ll.ts,
                                    ll.stream,
                                    &ll.line,
                                );
                                last_buf = buf;
                                last_drop = dropped;
                                batch.push(parsed);
                                if batch.len() >= MAX_LINES_PER_BATCH {
                                    let send_batch = std::mem::take(&mut batch);
                                    if flusher_chan.send(LogChannelFrame::Batch {
                                        stream_id: flusher_sid.clone(),
                                        lines: send_batch,
                                        total_buffered: last_buf,
                                        dropped_total: last_drop,
                                    }).is_err() {
                                        break;
                                    }
                                }
                            }
                            None => {
                                if !batch.is_empty() {
                                    let _ = flusher_chan.send(LogChannelFrame::Batch {
                                        stream_id: flusher_sid.clone(),
                                        lines: batch,
                                        total_buffered: last_buf,
                                        dropped_total: last_drop,
                                    });
                                }
                                break;
                            }
                        }
                    }
                    _ = ticker.tick() => {
                        if !batch.is_empty() {
                            let send_batch = std::mem::take(&mut batch);
                            if flusher_chan.send(LogChannelFrame::Batch {
                                stream_id: flusher_sid.clone(),
                                lines: send_batch,
                                total_buffered: last_buf,
                                dropped_total: last_drop,
                            }).is_err() {
                                break;
                            }
                        }
                    }
                }
            }
        });

        let _ = on_frame.send(LogChannelFrame::Status {
            stream_id: stream_id.clone(),
            state: "streaming".into(),
            message: None,
        });

        let q = AppLogQuery {
            tail: Some(tail.unwrap_or(500)),
            follow: Some(true),
            timestamps: Some(true),
            ..Default::default()
        };
        let res = client
            .app_logs_stream_cancellable(&app_id, &q, stop_rx, |ev| {
                if ev.event == "log"
                    && let Ok(parsed) = serde_json::from_str::<LogLine>(&ev.data)
                {
                    let _ = line_tx.try_send(parsed);
                }
            })
            .await;

        drop(line_tx);
        let _ = flusher.await;

        let (st, msg) = match res {
            Ok(()) => ("ended".into(), None),
            Err(e) => ("error".into(), Some(e.to_string())),
        };
        let _ = on_frame.send(LogChannelFrame::Status {
            stream_id,
            state: st,
            message: msg,
        });
    });

    Ok(())
}

#[tauri::command]
pub async fn start_log_benchmark(
    state: State<'_, DesktopState>,
    stream_id: String,
    lines_per_sec: Option<u32>,
    duration_secs: Option<u32>,
    on_frame: Channel<LogChannelFrame>,
) -> Result<(), String> {
    let rate = lines_per_sec.unwrap_or(5_000);
    let secs = duration_secs.unwrap_or(5);
    state
        .logs
        .spawn_benchmark(stream_id, rate, secs, move |frame| {
            on_frame.send(frame).is_ok()
        });
    Ok(())
}

#[tauri::command]
pub fn stop_log_stream(state: State<'_, DesktopState>, stream_id: String) {
    state.logs.stop_stream(&stream_id);
}

#[tauri::command]
pub fn query_log_buffer(
    state: State<'_, DesktopState>,
    stream_id: String,
    filter: Option<String>,
    stream_filter: Option<String>,
    offset: usize,
    limit: usize,
) -> LogQueryResult {
    state.logs.query(
        &stream_id,
        filter.as_deref(),
        stream_filter.as_deref(),
        offset,
        limit.clamp(1, 2000),
    )
}

#[tauri::command]
pub async fn get_perf_diagnostics(state: State<'_, DesktopState>) -> Result<PerfDiagnostics, String> {
    let server_count = state.registry.read().await.list().len();
    Ok(PerfDiagnostics {
        uptime_ms: state.started_at.elapsed().as_millis() as u64,
        server_count,
        log_buffers: state.logs.stats(),
        client_version: env!("CARGO_PKG_VERSION").to_owned(),
    })
}

// -------------------------------------------------- desktop-only features (§9)

#[tauri::command]
pub async fn start_deploy_logs(
    state: State<'_, DesktopState>,
    server_id: String,
    deployment_id: String,
    stream_id: String,
    on_frame: Channel<LogChannelFrame>,
) -> Result<(), String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    let engine = Arc::clone(&state.logs);
    let stop_rx = engine.register_stream(&stream_id);

    tokio::spawn(async move {
        let _ = on_frame.send(LogChannelFrame::Status {
            stream_id: stream_id.clone(),
            state: "streaming".into(),
            message: None,
        });
        let path = format!(
            "/api/v1/deployments/{}/logs",
            rig_client::segment(&deployment_id)
        );
        let mut seq_counter = 0u64;
        let res = client
            .events_cancellable(&path, None, stop_rx, |ev| {
                if ev.event == "log"
                    && let Ok(ll) = serde_json::from_str::<LogLine>(&ev.data)
                {
                    seq_counter += 1;
                    let seq = if ll.seq > 0 { ll.seq as u64 } else { seq_counter };
                    let (parsed, buf, dropped) =
                        engine.push_line(&stream_id, seq, ll.ts, ll.stream, &ll.line);
                    let _ = on_frame.send(LogChannelFrame::Batch {
                        stream_id: stream_id.clone(),
                        lines: vec![parsed],
                        total_buffered: buf,
                        dropped_total: dropped,
                    });
                }
            })
            .await;
        let (st, msg) = match res {
            Ok(()) => ("ended".into(), None),
            Err(e) => ("error".into(), Some(e.to_string())),
        };
        let _ = on_frame.send(LogChannelFrame::Status {
            stream_id,
            state: st,
            message: msg,
        });
    });

    Ok(())
}

#[tauri::command]
pub async fn set_env_var(
    state: State<'_, DesktopState>,
    server_id: String,
    app_id: String,
    req: SetEnvVarRequest,
) -> Result<EnvVar, String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    client
        .set_env(&app_id, &req)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn reveal_env_var(
    state: State<'_, DesktopState>,
    server_id: String,
    app_id: String,
    key: String,
) -> Result<RevealedEnvVar, String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    client
        .reveal_env(&app_id, &key)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_env_var(
    state: State<'_, DesktopState>,
    server_id: String,
    app_id: String,
    key: String,
) -> Result<(), String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    client
        .delete_env(&app_id, &key)
        .await
        .map_err(|e| e.to_string())
}

/// Discovers local Git repositories & deployable project folders on this machine.
#[tauri::command]
pub async fn discover_local_repos(
    extra_roots: Option<Vec<String>>,
) -> Result<Vec<DiscoveredRepo>, String> {
    tokio::task::spawn_blocking(move || discover_repos(extra_roots))
        .await
        .map_err(|e| e.to_string())
}

/// Parses dropped `.env` content locally in Rust and diffs against current server env vars.
#[tauri::command]
pub async fn preview_env_diff(
    state: State<'_, DesktopState>,
    server_id: String,
    app_id: String,
    dotenv_content: String,
) -> Result<EnvDiffPreview, String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    let current = client.list_env(&app_id).await.map_err(|e| e.to_string())?;
    Ok(diff_env(&dotenv_content, &current))
}

/// Applies diffed `.env` items in one atomic bulk request.
#[tauri::command]
pub async fn apply_env_diff(
    state: State<'_, DesktopState>,
    server_id: String,
    app_id: String,
    items: Vec<EnvDiffItem>,
) -> Result<Vec<EnvVar>, String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    let req = to_bulk_request(&items);
    if req.vars.is_empty() {
        return client.list_env(&app_id).await.map_err(|e| e.to_string());
    }
    client
        .set_env_bulk(&app_id, &req)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn inspect_local_git(path: String) -> Result<LocalGitStatus, String> {
    tokio::task::spawn_blocking(move || inspect_repo(&path))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn deploy_local_git(
    state: State<'_, DesktopState>,
    server_id: String,
    app_id: String,
    path: String,
    dockerfile_path: Option<String>,
) -> Result<Deployment, String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    deploy_local_checkout(&client, &app_id, &path, dockerfile_path)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn open_terminal(
    state: State<'_, DesktopState>,
    server_id: String,
    app_id: String,
    session_id: String,
    cols: u16,
    rows: u16,
    on_frame: Channel<TerminalFrame>,
) -> Result<(), String> {
    let (_, client) = client_for_server(&state, &server_id).await?;
    state
        .terminals
        .open(session_id, client, app_id, cols, rows, move |frame| {
            on_frame.send(frame).is_ok()
        })
        .await
}

#[tauri::command]
pub async fn write_terminal(
    state: State<'_, DesktopState>,
    session_id: String,
    data: Vec<u8>,
) -> Result<(), String> {
    state.terminals.write(&session_id, data).await
}

#[tauri::command]
pub async fn resize_terminal(
    state: State<'_, DesktopState>,
    session_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    state.terminals.resize(&session_id, cols, rows).await
}

#[tauri::command]
pub async fn close_terminal(
    state: State<'_, DesktopState>,
    session_id: String,
) -> Result<(), String> {
    state.terminals.close(&session_id).await;
    Ok(())
}
