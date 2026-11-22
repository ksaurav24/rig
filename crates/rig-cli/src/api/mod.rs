//! Typed HTTP + SSE client for the Rig API.

pub mod sse;

use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use reqwest::{Method, Response, StatusCode, header};
use rig_api::{
    AddDomainRequest, AddMemberRequest, AlertChannel, AlertTestResult, ApiToken, App, AppWebhook,
    AuditEntry, Backup, BulkSetEnvRequest, CreateAlertChannelRequest, CreateApiTokenRequest,
    CreateAppRequest, CreateDatabaseRequest, CreateProjectRequest, CreatedAlertChannel,
    CreatedApiToken, Database, DatabaseCredentials, DatabaseEngineInfo, Deployment, Domain, EnvVar,
    ErrorBody, GithubBranch, GithubRepo, GithubStatus, Me, Member, MemberWithPassword, MetricRange,
    MetricSeries, Project, RevealedEnvVar, SetEnvVarRequest, SystemInfo, UpdateAlertChannelRequest,
    UpdateAppRequest, UpdateMemberRequest, UpdateProjectRequest,
};
use serde::{Serialize, de::DeserializeOwned};

use crate::config::Context;
use crate::error::{CliError, CliResult};
use sse::{SseEvent, SseParser};

const SSE_BACKOFF_MS: [u64; 5] = [200, 500, 1000, 2000, 4000];

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

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base: String,
    token: String,
    verbose: bool,
    warn_skew: bool,
    skew_warned: Arc<AtomicBool>,
}

impl Client {
    pub fn new(ctx: Context) -> CliResult<Self> {
        Self::with_options(ctx, false, true)
    }

    pub fn with_options(ctx: Context, verbose: bool, warn_skew: bool) -> CliResult<Self> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("rig-cli/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            // Tokens must not follow a redirect to another host.
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            http,
            base: ctx.url.trim_end_matches('/').to_string(),
            token: ctx.token,
            verbose,
            warn_skew,
            skew_warned: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    /// Checks a reported server version string against this CLI's version and
    /// prints a one-time stderr warning if they differ.
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
                "rig: warning: CLI version {cli_version} differs from server version {server_version}"
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
    ) -> CliResult<Response> {
        let started = Instant::now();
        let url = format!("{}{path}", self.base);
        let mut req = self
            .http
            .request(method.clone(), &url)
            .bearer_auth(&self.token);
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
                CliError::network(format!(
                    "could not reach {}. Is the server running and the address right?",
                    self.base
                ))
            } else {
                CliError::from(clean)
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
        let status = res.status();
        let text = res.text().await.unwrap_or_default();
        let message = match serde_json::from_str::<ErrorBody>(&text) {
            Ok(body) => body.error.message,
            Err(_) => format!("the server answered {status}"),
        };
        Err(match status {
            StatusCode::UNAUTHORIZED => CliError::auth(format!(
                "{message}. The token may be expired or deleted. Sign in again with: rig login {}",
                self.base
            )),
            StatusCode::FORBIDDEN => CliError::auth(message),
            StatusCode::NOT_FOUND => CliError::not_found(message),
            StatusCode::CONFLICT => CliError::conflict(message),
            _ => CliError::message(message),
        })
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> CliResult<T> {
        decode(self.send(Method::GET, path, None::<&()>, false, None).await?).await
    }

    pub async fn post<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: Option<&B>,
    ) -> CliResult<T> {
        decode(self.send(Method::POST, path, body, false, None).await?).await
    }

    pub async fn post_empty(&self, path: &str) -> CliResult<()> {
        self.send(Method::POST, path, None::<&()>, false, None)
            .await
            .map(drop)
    }

    pub async fn patch<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> CliResult<T> {
        decode(self.send(Method::PATCH, path, Some(body), false, None).await?).await
    }

    pub async fn put<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> CliResult<T> {
        decode(self.send(Method::PUT, path, Some(body), false, None).await?).await
    }

    pub async fn delete(&self, path: &str) -> CliResult<()> {
        self.send(Method::DELETE, path, None::<&()>, false, None)
            .await
            .map(drop)
    }

    /// Streams a binary response into `dest` (or derives filename from `Content-Disposition`).
    /// Returns `(written_path, bytes_written)`.
    pub async fn download(&self, path: &str, dest: Option<&Path>) -> CliResult<(std::path::PathBuf, u64)> {
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
            .map_err(|e| CliError::network(format!("download interrupted: {}", e.without_url())))?
        {
            std::io::Write::write_all(&mut file, &chunk)?;
            total += chunk.len() as u64;
        }
        std::io::Write::flush(&mut file)?;
        Ok((out_path, total))
    }

    /// Reads an SSE stream once until the server closes it or emits `end`.
    pub async fn events(&self, path: &str, mut on_event: impl FnMut(SseEvent)) -> CliResult<()> {
        let mut res = self
            .send(Method::GET, path, None::<&()>, true, None)
            .await?;
        let mut parser = SseParser::default();
        while let Some(chunk) = res
            .chunk()
            .await
            .map_err(|e| CliError::network(format!("the log stream was interrupted: {}", e.without_url())))?
        {
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
    ) -> CliResult<()> {
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
                .send(Method::GET, &path, None::<&()>, true, last_event_id_str(&last_id))
                .await
            {
                Ok(r) => r,
                Err(CliError::Network(_)) if attempt < SSE_BACKOFF_MS.len() => {
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
                Some(e) => Err(CliError::network(format!(
                    "the log stream was interrupted and could not be resumed: {e}"
                ))),
                None => Ok(()),
            };
        }
    }

    // --------------------------------------------------------- system & auth

    pub async fn me(&self) -> CliResult<Me> {
        self.get("/api/v1/auth/me").await
    }

    pub async fn system(&self) -> CliResult<SystemInfo> {
        let info: SystemInfo = self.get("/api/v1/system").await?;
        self.check_version_skew(&info.version);
        Ok(info)
    }

    pub async fn list_tokens(&self) -> CliResult<Vec<ApiToken>> {
        self.get("/api/v1/tokens").await
    }

    pub async fn create_token(&self, req: &CreateApiTokenRequest) -> CliResult<CreatedApiToken> {
        self.post("/api/v1/tokens", Some(req)).await
    }

    pub async fn delete_token(&self, token_id: &str) -> CliResult<()> {
        self.delete(&format!("/api/v1/tokens/{}", segment(token_id)))
            .await
    }

    pub async fn list_audit(&self) -> CliResult<Vec<AuditEntry>> {
        self.get("/api/v1/audit").await
    }

    // -------------------------------------------------------------- projects

    pub async fn list_projects(&self) -> CliResult<Vec<Project>> {
        self.get("/api/v1/projects").await
    }

    pub async fn create_project(&self, req: &CreateProjectRequest) -> CliResult<Project> {
        self.post("/api/v1/projects", Some(req)).await
    }

    pub async fn update_project(
        &self,
        project_id: &str,
        req: &UpdateProjectRequest,
    ) -> CliResult<Project> {
        self.patch(&format!("/api/v1/projects/{}", segment(project_id)), req)
            .await
    }

    pub async fn delete_project(
        &self,
        project_id: &str,
        databases_action: Option<&str>,
        target_project_id: Option<&str>,
    ) -> CliResult<()> {
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

    pub async fn list_apps(&self, project_id: Option<&str>) -> CliResult<Vec<App>> {
        let path = match project_id {
            Some(id) => format!("/api/v1/apps?project_id={}", segment(id)),
            None => "/api/v1/apps".to_string(),
        };
        self.get(&path).await
    }

    pub async fn resolve_app_ref(&self, app_ref: &str) -> CliResult<App> {
        self.get(&format!("/api/v1/apps/resolve?ref={}", segment(app_ref)))
            .await
    }

    pub async fn create_app(&self, req: &CreateAppRequest) -> CliResult<App> {
        self.post("/api/v1/apps", Some(req)).await
    }

    pub async fn get_app(&self, app_id: &str) -> CliResult<App> {
        self.get(&format!("/api/v1/apps/{}", segment(app_id))).await
    }

    pub async fn update_app(&self, app_id: &str, req: &UpdateAppRequest) -> CliResult<App> {
        self.patch(&format!("/api/v1/apps/{}", segment(app_id)), req)
            .await
    }

    pub async fn delete_app(&self, app_id: &str) -> CliResult<()> {
        self.delete(&format!("/api/v1/apps/{}", segment(app_id)))
            .await
    }

    pub async fn deploy_app(&self, app_id: &str) -> CliResult<Deployment> {
        self.post(
            &format!("/api/v1/apps/{}/deploy", segment(app_id)),
            None::<&()>,
        )
        .await
    }

    pub async fn start_app(&self, app_id: &str) -> CliResult<Deployment> {
        self.post(
            &format!("/api/v1/apps/{}/start", segment(app_id)),
            None::<&()>,
        )
        .await
    }

    pub async fn restart_app(&self, app_id: &str) -> CliResult<Deployment> {
        self.post(
            &format!("/api/v1/apps/{}/restart", segment(app_id)),
            None::<&()>,
        )
        .await
    }

    pub async fn stop_app(&self, app_id: &str) -> CliResult<App> {
        self.post(
            &format!("/api/v1/apps/{}/stop", segment(app_id)),
            None::<&()>,
        )
        .await
    }

    pub async fn get_webhook(&self, app_id: &str) -> CliResult<AppWebhook> {
        self.get(&format!("/api/v1/apps/{}/webhook", segment(app_id)))
            .await
    }

    pub async fn rotate_webhook(&self, app_id: &str) -> CliResult<AppWebhook> {
        self.post(
            &format!("/api/v1/apps/{}/webhook/rotate", segment(app_id)),
            None::<&()>,
        )
        .await
    }

    // ----------------------------------------------------------- deployments

    pub async fn list_deployments(&self, app_id: &str) -> CliResult<Vec<Deployment>> {
        self.list_deployments_page(app_id, &DeploymentListQuery::default())
            .await
    }

    pub async fn list_deployments_page(
        &self,
        app_id: &str,
        q: &DeploymentListQuery,
    ) -> CliResult<Vec<Deployment>> {
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

    pub async fn get_deployment(&self, deployment_id: &str) -> CliResult<Deployment> {
        self.get(&format!("/api/v1/deployments/{}", segment(deployment_id)))
            .await
    }

    pub async fn rollback_deployment(&self, deployment_id: &str) -> CliResult<Deployment> {
        self.post(
            &format!("/api/v1/deployments/{}/rollback", segment(deployment_id)),
            None::<&()>,
        )
        .await
    }

    pub async fn cancel_deployment(&self, deployment_id: &str) -> CliResult<Deployment> {
        self.post(
            &format!("/api/v1/deployments/{}/cancel", segment(deployment_id)),
            None::<&()>,
        )
        .await
    }

    // -------------------------------------------------------------- env vars

    pub async fn list_env(&self, app_id: &str) -> CliResult<Vec<EnvVar>> {
        self.get(&format!("/api/v1/apps/{}/env", segment(app_id)))
            .await
    }

    pub async fn set_env(&self, app_id: &str, req: &SetEnvVarRequest) -> CliResult<EnvVar> {
        self.put(&format!("/api/v1/apps/{}/env", segment(app_id)), req)
            .await
    }

    pub async fn set_env_bulk(
        &self,
        app_id: &str,
        req: &BulkSetEnvRequest,
    ) -> CliResult<Vec<EnvVar>> {
        self.put(&format!("/api/v1/apps/{}/env/bulk", segment(app_id)), req)
            .await
    }

    pub async fn reveal_env(&self, app_id: &str, key: &str) -> CliResult<RevealedEnvVar> {
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

    pub async fn delete_env(&self, app_id: &str, key: &str) -> CliResult<()> {
        self.delete(&format!(
            "/api/v1/apps/{}/env/{}",
            segment(app_id),
            segment(key)
        ))
        .await
    }

    // --------------------------------------------------------------- domains

    pub async fn list_domains(&self, app_id: &str) -> CliResult<Vec<Domain>> {
        self.get(&format!("/api/v1/apps/{}/domains", segment(app_id)))
            .await
    }

    pub async fn add_domain(&self, app_id: &str, req: &AddDomainRequest) -> CliResult<Domain> {
        self.post(
            &format!("/api/v1/apps/{}/domains", segment(app_id)),
            Some(req),
        )
        .await
    }

    pub async fn delete_domain(&self, app_id: &str, domain_id: &str) -> CliResult<()> {
        self.delete(&format!(
            "/api/v1/apps/{}/domains/{}",
            segment(app_id),
            segment(domain_id)
        ))
        .await
    }

    // ---------------------------------------------------- databases & backups

    pub async fn list_databases(&self) -> CliResult<Vec<Database>> {
        self.list_databases_in(None).await
    }

    pub async fn list_databases_in(&self, project_id: Option<&str>) -> CliResult<Vec<Database>> {
        let path = match project_id {
            Some(id) => format!("/api/v1/databases?project_id={}", segment(id)),
            None => "/api/v1/databases".to_string(),
        };
        self.get(&path).await
    }

    pub async fn list_database_engines(&self) -> CliResult<Vec<DatabaseEngineInfo>> {
        self.get("/api/v1/databases/engines").await
    }

    pub async fn create_database(&self, req: &CreateDatabaseRequest) -> CliResult<Database> {
        self.post("/api/v1/databases", Some(req)).await
    }

    pub async fn get_database(&self, db_id: &str) -> CliResult<Database> {
        self.get(&format!("/api/v1/databases/{}", segment(db_id)))
            .await
    }

    pub async fn start_database(&self, db_id: &str) -> CliResult<Database> {
        self.post(
            &format!("/api/v1/databases/{}/start", segment(db_id)),
            None::<&()>,
        )
        .await
    }

    pub async fn stop_database(&self, db_id: &str) -> CliResult<Database> {
        self.post(
            &format!("/api/v1/databases/{}/stop", segment(db_id)),
            None::<&()>,
        )
        .await
    }

    pub async fn delete_database(&self, db_id: &str) -> CliResult<()> {
        self.delete(&format!("/api/v1/databases/{}", segment(db_id)))
            .await
    }

    pub async fn reveal_database(&self, db_id: &str) -> CliResult<DatabaseCredentials> {
        self.post(
            &format!("/api/v1/databases/{}/reveal", segment(db_id)),
            None::<&()>,
        )
        .await
    }

    pub async fn list_backups(&self, db_id: &str) -> CliResult<Vec<Backup>> {
        self.get(&format!("/api/v1/databases/{}/backups", segment(db_id)))
            .await
    }

    pub async fn create_backup(&self, db_id: &str) -> CliResult<Backup> {
        self.post(
            &format!("/api/v1/databases/{}/backups", segment(db_id)),
            None::<&()>,
        )
        .await
    }

    pub async fn delete_backup(&self, backup_id: &str) -> CliResult<()> {
        self.delete(&format!("/api/v1/backups/{}", segment(backup_id)))
            .await
    }

    pub async fn restore_backup(&self, backup_id: &str) -> CliResult<()> {
        self.post_empty(&format!("/api/v1/backups/{}/restore", segment(backup_id)))
            .await
    }

    pub async fn download_backup(
        &self,
        backup_id: &str,
        dest: Option<&Path>,
    ) -> CliResult<(std::path::PathBuf, u64)> {
        self.download(
            &format!("/api/v1/backups/{}/download", segment(backup_id)),
            dest,
        )
        .await
    }

    // --------------------------------------------------------------- metrics

    pub async fn host_metrics(&self, range: MetricRange) -> CliResult<MetricSeries> {
        self.get(&format!(
            "/api/v1/system/metrics?range={}",
            metric_range_str(range)
        ))
        .await
    }

    pub async fn app_metrics(&self, app_id: &str, range: MetricRange) -> CliResult<MetricSeries> {
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
    ) -> CliResult<MetricSeries> {
        self.get(&format!(
            "/api/v1/databases/{}/metrics?range={}",
            segment(db_id),
            metric_range_str(range)
        ))
        .await
    }

    // --------------------------------------------------------------- members

    pub async fn list_members(&self) -> CliResult<Vec<Member>> {
        self.get("/api/v1/members").await
    }

    pub async fn add_member(&self, req: &AddMemberRequest) -> CliResult<MemberWithPassword> {
        self.post("/api/v1/members", Some(req)).await
    }

    pub async fn update_member(
        &self,
        user_id: &str,
        req: &UpdateMemberRequest,
    ) -> CliResult<Member> {
        self.patch(&format!("/api/v1/members/{}", segment(user_id)), req)
            .await
    }

    pub async fn remove_member(&self, user_id: &str) -> CliResult<()> {
        self.delete(&format!("/api/v1/members/{}", segment(user_id)))
            .await
    }

    pub async fn reset_member_password(&self, user_id: &str) -> CliResult<MemberWithPassword> {
        self.post(
            &format!("/api/v1/members/{}/reset-password", segment(user_id)),
            None::<&()>,
        )
        .await
    }

    // ---------------------------------------------------------------- alerts

    pub async fn list_alert_channels(&self) -> CliResult<Vec<AlertChannel>> {
        self.get("/api/v1/alerts/channels").await
    }

    pub async fn create_alert_channel(
        &self,
        req: &CreateAlertChannelRequest,
    ) -> CliResult<CreatedAlertChannel> {
        self.post("/api/v1/alerts/channels", Some(req)).await
    }

    pub async fn update_alert_channel(
        &self,
        id: &str,
        req: &UpdateAlertChannelRequest,
    ) -> CliResult<AlertChannel> {
        self.patch(&format!("/api/v1/alerts/channels/{}", segment(id)), req)
            .await
    }

    pub async fn delete_alert_channel(&self, id: &str) -> CliResult<()> {
        self.delete(&format!("/api/v1/alerts/channels/{}", segment(id)))
            .await
    }

    pub async fn test_alert_channel(&self, id: &str) -> CliResult<AlertTestResult> {
        self.post(
            &format!("/api/v1/alerts/channels/{}/test", segment(id)),
            None::<&()>,
        )
        .await
    }

    // ---------------------------------------------------------------- github

    pub async fn github_status(&self) -> CliResult<GithubStatus> {
        self.get("/api/v1/github").await
    }

    pub async fn github_repos(&self) -> CliResult<Vec<GithubRepo>> {
        self.get("/api/v1/github/repos").await
    }

    pub async fn github_branches(&self, owner: &str, repo: &str) -> CliResult<Vec<GithubBranch>> {
        self.get(&format!(
            "/api/v1/github/repos/{}/{}/branches",
            segment(owner),
            segment(repo)
        ))
        .await
    }

    // --------------------------------------------------------------- streams

    pub async fn app_logs_stream(
        &self,
        app_id: &str,
        q: &AppLogQuery,
        on_event: impl FnMut(SseEvent),
    ) -> CliResult<()> {
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
        self.events(&path, on_event).await
    }

    pub async fn deployment_logs_stream(
        &self,
        deployment_id: &str,
        on_event: impl FnMut(SseEvent),
    ) -> CliResult<()> {
        self.events_resumable(
            &format!("/api/v1/deployments/{}/logs", segment(deployment_id)),
            on_event,
        )
        .await
    }

    /// Builds the `ws://` or `wss://` URL for an app's terminal WebSocket.
    pub fn terminal_ws_url(&self, app_id: &str, cols: u16, rows: u16) -> CliResult<String> {
        let ws_base = if let Some(rest) = self.base.strip_prefix("https://") {
            format!("wss://{rest}")
        } else if let Some(rest) = self.base.strip_prefix("http://") {
            format!("ws://{rest}")
        } else {
            return Err(CliError::message(format!(
                "unsupported server scheme in {}",
                self.base
            )));
        };
        Ok(format!(
            "{ws_base}/api/v1/apps/{}/terminal?cols={cols}&rows={rows}",
            segment(app_id)
        ))
    }
}

fn last_event_id_str(id: &Option<String>) -> Option<&str> {
    id.as_deref()
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
            // Reject path traversal components.
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

async fn decode<T: DeserializeOwned>(res: Response) -> CliResult<T> {
    res.json()
        .await
        .context("the server sent a reply this version of rig does not understand")
        .map_err(CliError::from)
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
