//! Shared typed HTTP, SSE, and WebSocket client for the Rig API.
//!
//! Used by `rig` (CLI + MCP) and `Rig Desktop` (Tauri Rust core), keeping auth
//! headers, retries, SSE resume (`Last-Event-ID` / `?after=`), TLS certificate
//! pinning, and version negotiation in a single crate.

pub mod error;
pub mod sse;
pub mod tls;

use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use reqwest::{Method, Response, StatusCode, header};
use rig_api::{
    AddDomainRequest, AddMemberRequest, AdoptExecuteRequest, AdoptExecuteResult, AdoptPreview,
    AdoptPreviewRequest, AlertChannel, AlertTestResult, ApiToken, App, AppWebhook, AuditEntry,
    Backup, BinaryDeployRequest, BulkSetEnvRequest, CheckBlobsRequest, CheckBlobsResponse,
    CreateAlertChannelRequest, CreateApiTokenRequest, CreateAppRequest, CreateDatabaseRequest,
    CreatePairingRequest, CreateProjectRequest, CreatedAlertChannel, CreatedApiToken, Database,
    DatabaseCredentials, DatabaseEngineInfo, Deployment, DeviceLoginRequest, DiscoveredContainer,
    Domain, EnvVar, ErrorBody, ExchangePairingRequest, GithubBranch, GithubManifest,
    GithubManifestComplete, GithubManifestRequest, GithubRepo, GithubStatus,
    LocalImageDeployRequest, LocalSourceDeployRequest, Me, Member, MemberWithPassword, MetricRange,
    MetricSeries, NetworkInfo, PairedDevice, PairingCode, Project, RevealedEnvVar, ServerEvent,
    ServerMeta, SetEnvVarRequest, SetupRequest, SetupStatus, StoredBlob, StoredBlobBatch,
    SystemInfo, UpdateAlertChannelRequest, UpdateAppRequest, UpdateMemberRequest,
    UpdateOrganizationRequest, UpdateProjectRequest,
};
use serde::{Serialize, de::DeserializeOwned};
use tokio::sync::watch;
use tokio_tungstenite::{Connector, MaybeTlsStream, WebSocketStream, tungstenite};

pub use error::{ClientError, ClientResult};
pub use sse::{SseEvent, SseParser};
pub use tls::{TlsProbe, probe_tls};

const SSE_BACKOFF_MS: [u64; 5] = [200, 500, 1000, 2000, 4000];
pub const MIN_API_VERSION: u32 = 1;

/// Query options for `GET /api/v1/apps/{id}/logs`.
#[derive(Debug, Clone, Default)]
pub struct AppLogQuery {
    pub tail: Option<u32>,
    pub follow: Option<bool>,
    pub since: Option<String>,
    pub timestamps: Option<bool>,
    pub grep: Option<String>,
}

/// Query options for `GET /api/v1/apps/{id}/deployments`.
#[derive(Debug, Clone, Default)]
pub struct DeploymentListQuery {
    pub limit: Option<u32>,
    pub before: Option<String>,
}

/// Configuration for building a [`Client`].
#[derive(Debug, Clone)]
pub struct ClientOptions {
    pub base_url: String,
    pub token: Option<String>,
    /// Optional SHA-256 leaf certificate fingerprint for `lan` / `--tls internal` servers.
    pub pinned_tls_sha256: Option<String>,
    pub user_agent: String,
    pub verbose: bool,
    pub warn_skew: bool,
}

impl ClientOptions {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            token: None,
            pinned_tls_sha256: None,
            user_agent: concat!("rig-client/", env!("CARGO_PKG_VERSION")).into(),
            verbose: false,
            warn_skew: false,
        }
    }
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base: String,
    token: Option<String>,
    pinned_tls_sha256: Option<String>,
    user_agent: String,
    verbose: bool,
    warn_skew: bool,
    skew_warned: Arc<AtomicBool>,
}

impl Client {
    pub fn build(opts: ClientOptions) -> ClientResult<Self> {
        let mut builder = reqwest::Client::builder()
            .user_agent(&opts.user_agent)
            .connect_timeout(Duration::from_secs(10))
            // Tokens must not follow a redirect to another host.
            .redirect(reqwest::redirect::Policy::none());

        if let Some(fp) = opts
            .pinned_tls_sha256
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let tls_cfg = tls::pinned_tls_config(fp)?;
            builder = builder.use_preconfigured_tls(tls_cfg);
        }

        let http = builder.build()?;
        Ok(Self {
            http,
            base: opts.base_url.trim_end_matches('/').to_string(),
            token: opts.token.filter(|t| !t.is_empty()),
            pinned_tls_sha256: opts.pinned_tls_sha256,
            user_agent: opts.user_agent,
            verbose: opts.verbose,
            warn_skew: opts.warn_skew,
            skew_warned: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn token(&self) -> &str {
        self.token.as_deref().unwrap_or("")
    }

    pub fn pinned_tls_sha256(&self) -> Option<&str> {
        self.pinned_tls_sha256.as_deref()
    }

    pub fn check_version_skew(&self, server_version: &str) {
        if !self.warn_skew {
            return;
        }
        let server_version = server_version.trim();
        let cli_version = env!("CARGO_PKG_VERSION");
        if server_version.is_empty() || server_version == cli_version {
            return;
        }
        if !self.skew_warned.swap(true, Ordering::Relaxed) {
            eprintln!(
                "rig: warning: client version {cli_version} differs from server version {server_version}"
            );
        }
    }

    async fn send<B: Serialize>(
        &self,
        method: Method,
        path: &str,
        body: Option<&B>,
        stream: bool,
        last_event_id: Option<&str>,
    ) -> ClientResult<Response> {
        let started = Instant::now();
        let url = format!("{}{path}", self.base);
        let mut req = self.http.request(method.clone(), &url);
        if let Some(tok) = &self.token {
            req = req.bearer_auth(tok);
        }
        if stream {
            // Avoid compressed SSE buffering on proxies and reqwest.
            req = req.header(header::ACCEPT_ENCODING, "identity");
            if let Some(id) = last_event_id {
                req = req.header("Last-Event-ID", id);
            }
        } else {
            req = req.timeout(Duration::from_secs(60));
        }
        if let Some(body) = body {
            req = req.json(body);
        }
        let res = req.send().await.map_err(|e| {
            let is_net = e.is_connect() || e.is_timeout();
            let clean = e.without_url();
            if self.verbose {
                eprintln!(
                    "rig: debug: {} {} -> error ({}ms): {clean}",
                    method,
                    path,
                    started.elapsed().as_millis(),
                );
            }
            if is_net {
                ClientError::network(format!(
                    "could not reach {}. Is the server running and the address right?",
                    self.base
                ))
            } else {
                ClientError::from(clean)
            }
        })?;
        if self.verbose {
            eprintln!(
                "rig: debug: {} {} -> {} ({}ms)",
                method,
                path,
                res.status().as_u16(),
                started.elapsed().as_millis()
            );
        }
        if let Some(ver) = res
            .headers()
            .get("x-rig-version")
            .and_then(|v| v.to_str().ok())
        {
            self.check_version_skew(ver);
        }
        if res.status().is_success() {
            return Ok(res);
        }
        Err(map_error_response(res, &self.base).await)
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> ClientResult<T> {
        decode(self.send(Method::GET, path, None::<&()>, false, None).await?).await
    }

    pub async fn post<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: Option<&B>,
    ) -> ClientResult<T> {
        decode(self.send(Method::POST, path, body, false, None).await?).await
    }

    pub async fn post_empty(&self, path: &str) -> ClientResult<()> {
        self.send(Method::POST, path, None::<&()>, false, None)
            .await
            .map(drop)
    }

    pub async fn patch<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> ClientResult<T> {
        decode(self.send(Method::PATCH, path, Some(body), false, None).await?).await
    }

    pub async fn put<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> ClientResult<T> {
        decode(self.send(Method::PUT, path, Some(body), false, None).await?).await
    }

    pub async fn delete(&self, path: &str) -> ClientResult<()> {
        self.send(Method::DELETE, path, None::<&()>, false, None)
            .await
            .map(drop)
    }

    async fn send_bytes<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        content_type: &str,
        bytes: Vec<u8>,
        timeout: Duration,
    ) -> ClientResult<T> {
        let started = Instant::now();
        let url = format!("{}{path}", self.base);
        let mut req = self
            .http
            .request(method.clone(), &url)
            .header(header::CONTENT_TYPE, content_type)
            .timeout(timeout)
            .body(bytes);
        if let Some(tok) = &self.token {
            req = req.bearer_auth(tok);
        }
        let res = req.send().await.map_err(|e| {
            let is_net = e.is_connect() || e.is_timeout();
            let clean = e.without_url();
            if is_net {
                ClientError::network(format!(
                    "could not reach {} while uploading: {clean}",
                    self.base
                ))
            } else {
                ClientError::from(clean)
            }
        })?;
        if self.verbose {
            eprintln!(
                "rig: debug: {} {} -> {} ({}ms)",
                method,
                path,
                res.status().as_u16(),
                started.elapsed().as_millis()
            );
        }
        if let Some(ver) = res
            .headers()
            .get("x-rig-version")
            .and_then(|v| v.to_str().ok())
        {
            self.check_version_skew(ver);
        }
        if res.status().is_success() {
            return decode(res).await;
        }
        Err(map_error_response(res, &self.base).await)
    }

    // --------------------------------------------------- meta, setup & pairing

    pub async fn meta(&self) -> ClientResult<ServerMeta> {
        self.get("/api/v1/meta").await
    }

    pub async fn setup_status(&self) -> ClientResult<SetupStatus> {
        self.get("/api/v1/setup").await
    }

    pub async fn complete_setup(&self, req: &SetupRequest) -> ClientResult<Me> {
        self.post("/api/v1/setup", Some(req)).await
    }

    pub async fn create_pairing(&self, req: &CreatePairingRequest) -> ClientResult<PairingCode> {
        self.post("/api/v1/devices/pair", Some(req)).await
    }

    pub async fn exchange_pairing(
        &self,
        req: &ExchangePairingRequest,
    ) -> ClientResult<PairedDevice> {
        self.post("/api/v1/devices/exchange", Some(req)).await
    }

    pub async fn device_login(&self, req: &DeviceLoginRequest) -> ClientResult<PairedDevice> {
        self.post("/api/v1/devices/login", Some(req)).await
    }

    pub async fn list_devices(&self) -> ClientResult<Vec<ApiToken>> {
        self.get("/api/v1/devices").await
    }

    pub async fn revoke_device(&self, device_id: &str) -> ClientResult<()> {
        self.delete(&format!("/api/v1/devices/{}", segment(device_id)))
            .await
    }

    // --------------------------------------------------------- local uploads

    pub async fn check_blobs(&self, hashes: &[String]) -> ClientResult<CheckBlobsResponse> {
        self.post(
            "/api/v1/uploads/blobs/check",
            Some(&CheckBlobsRequest {
                hashes: hashes.to_vec(),
            }),
        )
        .await
    }

    pub async fn upload_blob_zstd(
        &self,
        sha256: &str,
        zstd_bytes: Vec<u8>,
    ) -> ClientResult<StoredBlob> {
        self.send_bytes(
            Method::PUT,
            &format!("/api/v1/uploads/blobs/{}", segment(sha256)),
            "application/zstd",
            zstd_bytes,
            Duration::from_secs(600),
        )
        .await
    }

    pub async fn upload_blob_batch_zstd(
        &self,
        zstd_tar_bytes: Vec<u8>,
    ) -> ClientResult<StoredBlobBatch> {
        self.send_bytes(
            Method::POST,
            "/api/v1/uploads/blobs/batch",
            "application/zstd",
            zstd_tar_bytes,
            Duration::from_secs(600),
        )
        .await
    }

    pub async fn deploy_local_image(
        &self,
        app_id: &str,
        req: &LocalImageDeployRequest,
    ) -> ClientResult<Deployment> {
        let body = serde_json::to_vec(req)?;
        self.send_bytes(
            Method::POST,
            &format!("/api/v1/apps/{}/deploy/local-image", segment(app_id)),
            "application/json",
            body,
            Duration::from_secs(600),
        )
        .await
    }

    pub async fn deploy_local_source(
        &self,
        app_id: &str,
        req: &LocalSourceDeployRequest,
    ) -> ClientResult<Deployment> {
        self.post(
            &format!("/api/v1/apps/{}/deploy/local-source", segment(app_id)),
            Some(req),
        )
        .await
    }

    pub async fn deploy_binary(
        &self,
        app_id: &str,
        req: &BinaryDeployRequest,
    ) -> ClientResult<Deployment> {
        let body = serde_json::to_vec(req)?;
        self.send_bytes(
            Method::POST,
            &format!("/api/v1/apps/{}/deploy/binary", segment(app_id)),
            "application/json",
            body,
            Duration::from_secs(600),
        )
        .await
    }

    // ----------------------------------------------------------------- adopt

    pub async fn list_adoptable_containers(&self) -> ClientResult<Vec<DiscoveredContainer>> {
        self.get("/api/v1/adopt/containers").await
    }

    pub async fn preview_adopt(&self, req: &AdoptPreviewRequest) -> ClientResult<AdoptPreview> {
        self.post("/api/v1/adopt/preview", Some(req)).await
    }

    pub async fn execute_adopt(
        &self,
        req: &AdoptExecuteRequest,
    ) -> ClientResult<AdoptExecuteResult> {
        let body = serde_json::to_vec(req)?;
        self.send_bytes(
            Method::POST,
            "/api/v1/adopt/execute",
            "application/json",
            body,
            Duration::from_secs(300),
        )
        .await
    }

    /// Streams a binary response into `dest` (or derives filename from `Content-Disposition`).
    pub async fn download(
        &self,
        path: &str,
        dest: Option<&Path>,
    ) -> ClientResult<(std::path::PathBuf, u64)> {
        let mut res = self
            .send(Method::GET, path, None::<&()>, true, None)
            .await?;
        let out_path = match dest {
            Some(p) => p.to_path_buf(),
            None => {
                let suggested = res
                    .headers()
                    .get(header::CONTENT_DISPOSITION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(parse_content_disposition_filename)
                    .unwrap_or_else(|| "backup.bin".to_string());
                std::path::PathBuf::from(suggested)
            }
        };
        let mut file = std::fs::File::create(&out_path)
            .with_context(|| format!("creating {}", out_path.display()))?;
        let mut total = 0u64;
        while let Some(chunk) = res
            .chunk()
            .await
            .map_err(|e| ClientError::network(format!("download interrupted: {}", e.without_url())))?
        {
            std::io::Write::write_all(&mut file, &chunk)?;
            total += chunk.len() as u64;
        }
        std::io::Write::flush(&mut file)?;
        Ok((out_path, total))
    }

    /// Reads an SSE stream once until the server closes it or emits `end`.
    pub async fn events(
        &self,
        path: &str,
        mut on_event: impl FnMut(SseEvent),
    ) -> ClientResult<()> {
        let mut res = self
            .send(Method::GET, path, None::<&()>, true, None)
            .await?;
        let mut parser = SseParser::default();
        while let Some(chunk) = res.chunk().await.map_err(|e| {
            ClientError::network(format!("the log stream was interrupted: {}", e.without_url()))
        })? {
            let mut ended = false;
            parser.push(&chunk, |ev| {
                ended |= ev.event == "end";
                on_event(ev);
            });
            if ended {
                break;
            }
        }
        Ok(())
    }

    /// Reads an SSE stream once with cooperative cancellation via `stop_rx`.
    pub async fn events_cancellable(
        &self,
        path: &str,
        last_event_id: Option<&str>,
        mut stop_rx: watch::Receiver<bool>,
        mut on_event: impl FnMut(SseEvent),
    ) -> ClientResult<()> {
        let mut res = tokio::select! {
            _ = stop_rx.wait_for(|&s| s) => return Ok(()),
            r = self.send(Method::GET, path, None::<&()>, true, last_event_id) => r?,
        };
        let mut parser = SseParser::default();
        loop {
            let chunk = tokio::select! {
                _ = stop_rx.wait_for(|&s| s) => return Ok(()),
                c = res.chunk() => c.map_err(|e| {
                    ClientError::network(format!("stream interrupted: {}", e.without_url()))
                })?,
            };
            let Some(chunk) = chunk else {
                break;
            };
            let mut ended = false;
            parser.push(&chunk, |ev| {
                ended |= ev.event == "end";
                on_event(ev);
            });
            if ended {
                break;
            }
        }
        Ok(())
    }

    /// Reads a deployment SSE stream, automatically resuming with `Last-Event-ID`
    /// and `?after=<seq>` if the connection drops before an `end` event arrives.
    pub async fn events_resumable(
        &self,
        base_path: &str,
        mut on_event: impl FnMut(SseEvent),
    ) -> ClientResult<()> {
        let mut last_id: Option<String> = None;
        let mut attempt: usize = 0;

        loop {
            let path = match &last_id {
                Some(id) => {
                    let sep = if base_path.contains('?') { '&' } else { '?' };
                    format!("{base_path}{sep}after={}", segment(id))
                }
                None => base_path.to_string(),
            };

            let mut res = match self
                .send(Method::GET, &path, None::<&()>, true, last_id.as_deref())
                .await
            {
                Ok(r) => r,
                Err(ClientError::Network(_)) if attempt < SSE_BACKOFF_MS.len() => {
                    let delay = Duration::from_millis(SSE_BACKOFF_MS[attempt]);
                    attempt += 1;
                    tokio::time::sleep(delay).await;
                    continue;
                }
                Err(e) => return Err(e),
            };

            let mut parser = SseParser::default();
            let mut ended = false;
            let mut stream_err: Option<String> = None;

            loop {
                match res.chunk().await {
                    Ok(Some(chunk)) => {
                        attempt = 0;
                        parser.push(&chunk, |ev| {
                            if let Some(id) = &ev.id {
                                last_id = Some(id.clone());
                            }
                            ended |= ev.event == "end";
                            on_event(ev);
                        });
                        if ended {
                            return Ok(());
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        stream_err = Some(e.without_url().to_string());
                        break;
                    }
                }
            }

            if attempt < SSE_BACKOFF_MS.len() {
                let delay = Duration::from_millis(SSE_BACKOFF_MS[attempt]);
                attempt += 1;
                tokio::time::sleep(delay).await;
                continue;
            }

            return match stream_err {
                Some(e) => Err(ClientError::network(format!(
                    "the log stream was interrupted and could not be resumed: {e}"
                ))),
                None => Ok(()),
            };
        }
    }

    /// Streams server-wide events (`GET /api/v1/events`) with `Last-Event-ID`
    /// resume until `stop_rx` signals shutdown or an auth error occurs.
    pub async fn server_events_stream(
        &self,
        after: Option<u64>,
        stop_rx: watch::Receiver<bool>,
        mut on_event: impl FnMut(ServerEvent),
    ) -> ClientResult<Option<u64>> {
        let mut last_id = after;
        let path = match last_id {
            Some(id) if id > 0 => format!("/api/v1/events?after={id}"),
            _ => "/api/v1/events".to_string(),
        };
        let last_hdr = last_id.filter(|&id| id > 0).map(|id| id.to_string());
        self.events_cancellable(&path, last_hdr.as_deref(), stop_rx, |ev| {
            if let Ok(parsed) = serde_json::from_str::<ServerEvent>(&ev.data) {
                last_id = Some(parsed.id);
                on_event(parsed);
            }
        })
        .await?;
        Ok(last_id)
    }

    // --------------------------------------------------------- system & auth

    pub async fn me(&self) -> ClientResult<Me> {
        self.get("/api/v1/auth/me").await
    }

    pub async fn update_organization(
        &self,
        req: &UpdateOrganizationRequest,
    ) -> ClientResult<rig_api::Organization> {
        self.patch("/api/v1/org", req).await
    }

    pub async fn system(&self) -> ClientResult<SystemInfo> {
        let info: SystemInfo = self.get("/api/v1/system").await?;
        self.check_version_skew(&info.version);
        Ok(info)
    }

    pub async fn network(&self) -> ClientResult<NetworkInfo> {
        self.get("/api/v1/system/network").await
    }

    pub async fn list_tokens(&self) -> ClientResult<Vec<ApiToken>> {
        self.get("/api/v1/tokens").await
    }

    pub async fn create_token(&self, req: &CreateApiTokenRequest) -> ClientResult<CreatedApiToken> {
        self.post("/api/v1/tokens", Some(req)).await
    }

    pub async fn delete_token(&self, token_id: &str) -> ClientResult<()> {
        self.delete(&format!("/api/v1/tokens/{}", segment(token_id)))
            .await
    }

    pub async fn list_audit(&self) -> ClientResult<Vec<AuditEntry>> {
        self.get("/api/v1/audit").await
    }

    // -------------------------------------------------------------- projects

    pub async fn list_projects(&self) -> ClientResult<Vec<Project>> {
        self.get("/api/v1/projects").await
    }

    pub async fn create_project(&self, req: &CreateProjectRequest) -> ClientResult<Project> {
        self.post("/api/v1/projects", Some(req)).await
    }

    pub async fn update_project(
        &self,
        project_id: &str,
        req: &UpdateProjectRequest,
    ) -> ClientResult<Project> {
        self.patch(&format!("/api/v1/projects/{}", segment(project_id)), req)
            .await
    }

    pub async fn delete_project(
        &self,
        project_id: &str,
        databases_action: Option<&str>,
        target_project_id: Option<&str>,
    ) -> ClientResult<()> {
        let mut path = format!("/api/v1/projects/{}", segment(project_id));
        let mut params = Vec::new();
        if let Some(action) = databases_action {
            params.push(format!("databases={}", segment(action)));
        }
        if let Some(target) = target_project_id {
            params.push(format!("target={}", segment(target)));
        }
        if !params.is_empty() {
            path.push('?');
            path.push_str(&params.join("&"));
        }
        self.delete(&path).await
    }

    // ------------------------------------------------------------------ apps

    pub async fn list_apps(&self, project_id: Option<&str>) -> ClientResult<Vec<App>> {
        let path = match project_id {
            Some(id) => format!("/api/v1/apps?project_id={}", segment(id)),
            None => "/api/v1/apps".to_string(),
        };
        self.get(&path).await
    }

    pub async fn resolve_app_ref(&self, app_ref: &str) -> ClientResult<App> {
        self.get(&format!("/api/v1/apps/resolve?ref={}", segment(app_ref)))
            .await
    }

    pub async fn create_app(&self, req: &CreateAppRequest) -> ClientResult<App> {
        self.post("/api/v1/apps", Some(req)).await
    }

    pub async fn get_app(&self, app_id: &str) -> ClientResult<App> {
        self.get(&format!("/api/v1/apps/{}", segment(app_id))).await
    }

    pub async fn update_app(&self, app_id: &str, req: &UpdateAppRequest) -> ClientResult<App> {
        self.patch(&format!("/api/v1/apps/{}", segment(app_id)), req)
            .await
    }

    pub async fn delete_app(&self, app_id: &str) -> ClientResult<()> {
        self.delete(&format!("/api/v1/apps/{}", segment(app_id)))
            .await
    }

    pub async fn deploy_app(&self, app_id: &str) -> ClientResult<Deployment> {
        self.post(
            &format!("/api/v1/apps/{}/deploy", segment(app_id)),
            None::<&()>,
        )
        .await
    }

    pub async fn start_app(&self, app_id: &str) -> ClientResult<Deployment> {
        self.post(
            &format!("/api/v1/apps/{}/start", segment(app_id)),
            None::<&()>,
        )
        .await
    }

    pub async fn restart_app(&self, app_id: &str) -> ClientResult<Deployment> {
        self.post(
            &format!("/api/v1/apps/{}/restart", segment(app_id)),
            None::<&()>,
        )
        .await
    }

    pub async fn stop_app(&self, app_id: &str) -> ClientResult<App> {
        self.post(
            &format!("/api/v1/apps/{}/stop", segment(app_id)),
            None::<&()>,
        )
        .await
    }

    pub async fn get_webhook(&self, app_id: &str) -> ClientResult<AppWebhook> {
        self.get(&format!("/api/v1/apps/{}/webhook", segment(app_id)))
            .await
    }

    pub async fn rotate_webhook(&self, app_id: &str) -> ClientResult<AppWebhook> {
        self.post(
            &format!("/api/v1/apps/{}/webhook/rotate", segment(app_id)),
            None::<&()>,
        )
        .await
    }

    // ----------------------------------------------------------- deployments

    pub async fn list_deployments(&self, app_id: &str) -> ClientResult<Vec<Deployment>> {
        self.list_deployments_page(app_id, &DeploymentListQuery::default())
            .await
    }

    pub async fn list_deployments_page(
        &self,
        app_id: &str,
        q: &DeploymentListQuery,
    ) -> ClientResult<Vec<Deployment>> {
        let mut path = format!("/api/v1/apps/{}/deployments", segment(app_id));
        let mut params = Vec::new();
        if let Some(limit) = q.limit {
            params.push(format!("limit={limit}"));
        }
        if let Some(before) = q.before.as_deref().filter(|s| !s.is_empty()) {
            params.push(format!("before={}", segment(before)));
        }
        if !params.is_empty() {
            path.push('?');
            path.push_str(&params.join("&"));
        }
        self.get(&path).await
    }

    pub async fn get_deployment(&self, deployment_id: &str) -> ClientResult<Deployment> {
        self.get(&format!("/api/v1/deployments/{}", segment(deployment_id)))
            .await
    }

    pub async fn rollback_deployment(&self, deployment_id: &str) -> ClientResult<Deployment> {
        self.post(
            &format!("/api/v1/deployments/{}/rollback", segment(deployment_id)),
            None::<&()>,
        )
        .await
    }

    pub async fn cancel_deployment(&self, deployment_id: &str) -> ClientResult<Deployment> {
        self.post(
            &format!("/api/v1/deployments/{}/cancel", segment(deployment_id)),
            None::<&()>,
        )
        .await
    }

    // -------------------------------------------------------------- env vars

    pub async fn list_env(&self, app_id: &str) -> ClientResult<Vec<EnvVar>> {
        self.get(&format!("/api/v1/apps/{}/env", segment(app_id)))
            .await
    }

    pub async fn set_env(&self, app_id: &str, req: &SetEnvVarRequest) -> ClientResult<EnvVar> {
        self.put(&format!("/api/v1/apps/{}/env", segment(app_id)), req)
            .await
    }

    pub async fn set_env_bulk(
        &self,
        app_id: &str,
        req: &BulkSetEnvRequest,
    ) -> ClientResult<Vec<EnvVar>> {
        self.put(&format!("/api/v1/apps/{}/env/bulk", segment(app_id)), req)
            .await
    }

    pub async fn reveal_env(&self, app_id: &str, key: &str) -> ClientResult<RevealedEnvVar> {
        self.post(
            &format!(
                "/api/v1/apps/{}/env/{}/reveal",
                segment(app_id),
                segment(key)
            ),
            None::<&()>,
        )
        .await
    }

    pub async fn delete_env(&self, app_id: &str, key: &str) -> ClientResult<()> {
        self.delete(&format!(
            "/api/v1/apps/{}/env/{}",
            segment(app_id),
            segment(key)
        ))
        .await
    }

    // --------------------------------------------------------------- domains

    pub async fn list_domains(&self, app_id: &str) -> ClientResult<Vec<Domain>> {
        self.get(&format!("/api/v1/apps/{}/domains", segment(app_id)))
            .await
    }

    pub async fn add_domain(&self, app_id: &str, req: &AddDomainRequest) -> ClientResult<Domain> {
        self.post(
            &format!("/api/v1/apps/{}/domains", segment(app_id)),
            Some(req),
        )
        .await
    }

    pub async fn delete_domain(&self, app_id: &str, domain_id: &str) -> ClientResult<()> {
        self.delete(&format!(
            "/api/v1/apps/{}/domains/{}",
            segment(app_id),
            segment(domain_id)
        ))
        .await
    }

    // ---------------------------------------------------- databases & backups

    pub async fn list_databases(&self) -> ClientResult<Vec<Database>> {
        self.list_databases_in(None).await
    }

    pub async fn list_databases_in(&self, project_id: Option<&str>) -> ClientResult<Vec<Database>> {
        let path = match project_id {
            Some(id) => format!("/api/v1/databases?project_id={}", segment(id)),
            None => "/api/v1/databases".to_string(),
        };
        self.get(&path).await
    }

    pub async fn list_database_engines(&self) -> ClientResult<Vec<DatabaseEngineInfo>> {
        self.get("/api/v1/databases/engines").await
    }

    pub async fn create_database(&self, req: &CreateDatabaseRequest) -> ClientResult<Database> {
        self.post("/api/v1/databases", Some(req)).await
    }

    pub async fn get_database(&self, db_id: &str) -> ClientResult<Database> {
        self.get(&format!("/api/v1/databases/{}", segment(db_id)))
            .await
    }

    pub async fn start_database(&self, db_id: &str) -> ClientResult<Database> {
        self.post(
            &format!("/api/v1/databases/{}/start", segment(db_id)),
            None::<&()>,
        )
        .await
    }

    pub async fn stop_database(&self, db_id: &str) -> ClientResult<Database> {
        self.post(
            &format!("/api/v1/databases/{}/stop", segment(db_id)),
            None::<&()>,
        )
        .await
    }

    pub async fn delete_database(&self, db_id: &str) -> ClientResult<()> {
        self.delete(&format!("/api/v1/databases/{}", segment(db_id)))
            .await
    }

    pub async fn reveal_database(&self, db_id: &str) -> ClientResult<DatabaseCredentials> {
        self.post(
            &format!("/api/v1/databases/{}/reveal", segment(db_id)),
            None::<&()>,
        )
        .await
    }

    pub async fn list_backups(&self, db_id: &str) -> ClientResult<Vec<Backup>> {
        self.get(&format!("/api/v1/databases/{}/backups", segment(db_id)))
            .await
    }

    pub async fn create_backup(&self, db_id: &str) -> ClientResult<Backup> {
        self.post(
            &format!("/api/v1/databases/{}/backups", segment(db_id)),
            None::<&()>,
        )
        .await
    }

    pub async fn delete_backup(&self, backup_id: &str) -> ClientResult<()> {
        self.delete(&format!("/api/v1/backups/{}", segment(backup_id)))
            .await
    }

    pub async fn restore_backup(&self, backup_id: &str) -> ClientResult<()> {
        self.post_empty(&format!("/api/v1/backups/{}/restore", segment(backup_id)))
            .await
    }

    pub async fn download_backup(
        &self,
        backup_id: &str,
        dest: Option<&Path>,
    ) -> ClientResult<(std::path::PathBuf, u64)> {
        self.download(
            &format!("/api/v1/backups/{}/download", segment(backup_id)),
            dest,
        )
        .await
    }

    // --------------------------------------------------------------- metrics

    pub async fn host_metrics(&self, range: MetricRange) -> ClientResult<MetricSeries> {
        self.get(&format!(
            "/api/v1/system/metrics?range={}",
            metric_range_str(range)
        ))
        .await
    }

    pub async fn app_metrics(
        &self,
        app_id: &str,
        range: MetricRange,
    ) -> ClientResult<MetricSeries> {
        self.get(&format!(
            "/api/v1/apps/{}/metrics?range={}",
            segment(app_id),
            metric_range_str(range)
        ))
        .await
    }

    pub async fn database_metrics(
        &self,
        db_id: &str,
        range: MetricRange,
    ) -> ClientResult<MetricSeries> {
        self.get(&format!(
            "/api/v1/databases/{}/metrics?range={}",
            segment(db_id),
            metric_range_str(range)
        ))
        .await
    }

    // --------------------------------------------------------------- members

    pub async fn list_members(&self) -> ClientResult<Vec<Member>> {
        self.get("/api/v1/members").await
    }

    pub async fn add_member(&self, req: &AddMemberRequest) -> ClientResult<MemberWithPassword> {
        self.post("/api/v1/members", Some(req)).await
    }

    pub async fn update_member(
        &self,
        user_id: &str,
        req: &UpdateMemberRequest,
    ) -> ClientResult<Member> {
        self.patch(&format!("/api/v1/members/{}", segment(user_id)), req)
            .await
    }

    pub async fn remove_member(&self, user_id: &str) -> ClientResult<()> {
        self.delete(&format!("/api/v1/members/{}", segment(user_id)))
            .await
    }

    pub async fn reset_member_password(&self, user_id: &str) -> ClientResult<MemberWithPassword> {
        self.post(
            &format!("/api/v1/members/{}/reset-password", segment(user_id)),
            None::<&()>,
        )
        .await
    }

    // ---------------------------------------------------------------- alerts

    pub async fn list_alert_channels(&self) -> ClientResult<Vec<AlertChannel>> {
        self.get("/api/v1/alerts/channels").await
    }

    pub async fn create_alert_channel(
        &self,
        req: &CreateAlertChannelRequest,
    ) -> ClientResult<CreatedAlertChannel> {
        self.post("/api/v1/alerts/channels", Some(req)).await
    }

    pub async fn update_alert_channel(
        &self,
        id: &str,
        req: &UpdateAlertChannelRequest,
    ) -> ClientResult<AlertChannel> {
        self.patch(&format!("/api/v1/alerts/channels/{}", segment(id)), req)
            .await
    }

    pub async fn delete_alert_channel(&self, id: &str) -> ClientResult<()> {
        self.delete(&format!("/api/v1/alerts/channels/{}", segment(id)))
            .await
    }

    pub async fn test_alert_channel(&self, id: &str) -> ClientResult<AlertTestResult> {
        self.post(
            &format!("/api/v1/alerts/channels/{}/test", segment(id)),
            None::<&()>,
        )
        .await
    }

    // ---------------------------------------------------------------- github

    pub async fn github_status(&self) -> ClientResult<GithubStatus> {
        self.get("/api/v1/github").await
    }

    pub async fn github_manifest(
        &self,
        req: &GithubManifestRequest,
    ) -> ClientResult<GithubManifest> {
        self.post("/api/v1/github/manifest", Some(req)).await
    }

    pub async fn github_manifest_complete(
        &self,
        req: &GithubManifestComplete,
    ) -> ClientResult<GithubStatus> {
        self.post("/api/v1/github/manifest/complete", Some(req))
            .await
    }

    pub async fn github_disconnect(&self) -> ClientResult<()> {
        self.delete("/api/v1/github").await
    }

    pub async fn github_repos(&self) -> ClientResult<Vec<GithubRepo>> {
        self.get("/api/v1/github/repos").await
    }

    pub async fn github_branches(
        &self,
        owner: &str,
        repo: &str,
    ) -> ClientResult<Vec<GithubBranch>> {
        self.get(&format!(
            "/api/v1/github/repos/{}/{}/branches",
            segment(owner),
            segment(repo)
        ))
        .await
    }

    // --------------------------------------------------------------- streams

    pub fn app_logs_path(app_id: &str, q: &AppLogQuery) -> String {
        let mut path = format!("/api/v1/apps/{}/logs", segment(app_id));
        let mut params = Vec::new();
        if let Some(tail) = q.tail {
            params.push(format!("tail={tail}"));
        }
        if let Some(follow) = q.follow {
            params.push(format!("follow={follow}"));
        }
        if let Some(since) = q.since.as_deref().filter(|s| !s.is_empty()) {
            params.push(format!("since={}", segment(since)));
        }
        if let Some(ts) = q.timestamps {
            params.push(format!("timestamps={ts}"));
        }
        if let Some(grep) = q.grep.as_deref().filter(|s| !s.is_empty()) {
            params.push(format!("grep={}", segment(grep)));
        }
        if !params.is_empty() {
            path.push('?');
            path.push_str(&params.join("&"));
        }
        path
    }

    pub async fn app_logs_stream(
        &self,
        app_id: &str,
        q: &AppLogQuery,
        on_event: impl FnMut(SseEvent),
    ) -> ClientResult<()> {
        let path = Self::app_logs_path(app_id, q);
        self.events(&path, on_event).await
    }

    pub async fn app_logs_stream_cancellable(
        &self,
        app_id: &str,
        q: &AppLogQuery,
        stop_rx: watch::Receiver<bool>,
        on_event: impl FnMut(SseEvent),
    ) -> ClientResult<()> {
        let path = Self::app_logs_path(app_id, q);
        self.events_cancellable(&path, None, stop_rx, on_event)
            .await
    }

    pub async fn deployment_logs_stream(
        &self,
        deployment_id: &str,
        on_event: impl FnMut(SseEvent),
    ) -> ClientResult<()> {
        self.deployment_logs_stream_with_tail(deployment_id, None, on_event)
            .await
    }

    pub async fn deployment_logs_stream_with_tail(
        &self,
        deployment_id: &str,
        tail: Option<usize>,
        on_event: impl FnMut(SseEvent),
    ) -> ClientResult<()> {
        let path = match tail {
            Some(n) if n > 0 => format!(
                "/api/v1/deployments/{}/logs?tail={}",
                segment(deployment_id),
                n.min(10_000)
            ),
            _ => format!("/api/v1/deployments/{}/logs", segment(deployment_id)),
        };
        self.events_resumable(&path, on_event).await
    }

    /// Builds the `ws://` or `wss://` URL for an app's terminal WebSocket.
    pub fn terminal_ws_url(&self, app_id: &str, cols: u16, rows: u16) -> ClientResult<String> {
        let ws_base = if let Some(rest) = self.base.strip_prefix("https://") {
            format!("wss://{rest}")
        } else if let Some(rest) = self.base.strip_prefix("http://") {
            format!("ws://{rest}")
        } else {
            return Err(ClientError::message(format!(
                "unsupported server scheme in {}",
                self.base
            )));
        };
        Ok(format!(
            "{ws_base}/api/v1/apps/{}/terminal?cols={cols}&rows={rows}",
            segment(app_id)
        ))
    }

    /// Connects an authenticated container terminal WebSocket, honoring any
    /// pinned TLS certificate fingerprint configured on this client.
    pub async fn connect_terminal_ws(
        &self,
        app_id: &str,
        cols: u16,
        rows: u16,
    ) -> ClientResult<WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>> {
        let url = self.terminal_ws_url(app_id, cols, rows)?;
        let mut req = tungstenite::client::IntoClientRequest::into_client_request(&url)
            .map_err(|e| ClientError::message(format!("building terminal WS request: {e}")))?;
        if let Some(tok) = &self.token {
            req.headers_mut().insert(
                header::AUTHORIZATION,
                header::HeaderValue::from_str(&format!("Bearer {tok}"))
                    .map_err(|e| ClientError::message(format!("invalid bearer token: {e}")))?,
            );
        }
        if let Ok(ua) = header::HeaderValue::from_str(&self.user_agent) {
            req.headers_mut().insert(header::USER_AGENT, ua);
        }

        let connector = if let Some(fp) = self
            .pinned_tls_sha256
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(Connector::Rustls(Arc::new(tls::pinned_tls_config(fp)?)))
        } else {
            None
        };

        let (ws, _) = tokio_tungstenite::connect_async_tls_with_config(req, None, false, connector)
            .await
            .map_err(|e| ClientError::network(format!("connecting terminal websocket: {e}")))?;
        Ok(ws)
    }
}

async fn map_error_response(res: Response, base: &str) -> ClientError {
    let status = res.status();
    let text = res.text().await.unwrap_or_default();
    let parsed = serde_json::from_str::<ErrorBody>(&text).ok();
    let (code, message, challenge_token) = match parsed {
        Some(body) => (body.error.code, body.error.message, body.error.challenge_token),
        None => ("error".to_string(), format!("the server answered {status}"), None),
    };
    if status == StatusCode::UNAUTHORIZED && code == "second_factor_required" {
        return ClientError::SecondFactorRequired {
            message,
            challenge_token,
        };
    }
    match status {
        StatusCode::UNAUTHORIZED => ClientError::auth(format!(
            "{message}. The token may be expired or revoked ({base})."
        )),
        StatusCode::FORBIDDEN => ClientError::auth(message),
        StatusCode::NOT_FOUND => ClientError::not_found(message),
        StatusCode::CONFLICT => ClientError::conflict(message),
        StatusCode::TOO_MANY_REQUESTS => ClientError::rate_limited(message),
        _ => ClientError::Api {
            status: status.as_u16(),
            code,
            message,
        },
    }
}

pub fn metric_range_str(range: MetricRange) -> &'static str {
    match range {
        MetricRange::Hour => "1h",
        MetricRange::SixHours => "6h",
        MetricRange::Day => "24h",
    }
}

fn parse_content_disposition_filename(header_val: &str) -> Option<String> {
    for part in header_val.split(';') {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix("filename=") {
            let name = rest.trim().trim_matches('"');
            let clean = std::path::Path::new(name)
                .file_name()
                .and_then(|s| s.to_str())
                .filter(|s| !s.is_empty());
            if let Some(clean) = clean {
                return Some(clean.to_string());
            }
        }
    }
    None
}

async fn decode<T: DeserializeOwned>(res: Response) -> ClientResult<T> {
    res.json()
        .await
        .context("the server sent a reply this version of rig does not understand")
        .map_err(ClientError::from)
}

/// Percent-encodes one path segment or query value.
pub fn segment(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for b in raw.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_and_filenames() {
        assert_eq!(segment("API_KEY"), "API_KEY");
        assert_eq!(segment("a/b c"), "a%2Fb%20c");
        assert_eq!(
            parse_content_disposition_filename("attachment; filename=\"db-2026.sql.gz\""),
            Some("db-2026.sql.gz".into())
        );
        assert_eq!(
            parse_content_disposition_filename("attachment; filename=\"../../etc/passwd\""),
            Some("passwd".into())
        );
    }
}
