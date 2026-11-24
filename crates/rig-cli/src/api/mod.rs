//! Thin CLI adapter over [`rig_client::Client`], mapping [`rig_client::ClientError`]
//! into [`crate::error::CliError`] exit codes.

pub mod sse;

use std::path::{Path, PathBuf};

use rig_api::{
    AddDomainRequest, AddMemberRequest, AdoptExecuteRequest, AdoptExecuteResult, AdoptPreview,
    AdoptPreviewRequest, AlertChannel, AlertTestResult, ApiToken, App, AppWebhook, AuditEntry,
    Backup, BinaryDeployRequest, BulkSetEnvRequest, CheckBlobsResponse, CreateAlertChannelRequest,
    CreateApiTokenRequest, CreateAppRequest, CreateDatabaseRequest, CreatePairingRequest,
    CreateProjectRequest, CreatedAlertChannel, CreatedApiToken, Database, DatabaseCredentials,
    DatabaseEngineInfo, Deployment, DiscoveredContainer, Domain, EnvVar, GithubBranch, GithubRepo,
    GithubStatus, LocalImageDeployRequest, LocalSourceDeployRequest, Me, Member,
    MemberWithPassword, MetricRange, MetricSeries, PairingCode, Project, RevealedEnvVar,
    SetEnvVarRequest, StoredBlob, StoredBlobBatch, SystemInfo, UpdateAlertChannelRequest,
    UpdateAppRequest, UpdateMemberRequest, UpdateProjectRequest,
};
pub use rig_client::{AppLogQuery, DeploymentListQuery, metric_range_str, segment};
use rig_client::{Client as InnerClient, ClientOptions};
use serde::{Serialize, de::DeserializeOwned};

use crate::config::Context;
use crate::error::CliResult;
use sse::SseEvent;

#[derive(Clone)]
pub struct Client {
    inner: InnerClient,
}

impl Client {
    pub fn new(ctx: Context) -> CliResult<Self> {
        Self::with_options(ctx, false, true)
    }

    pub fn with_options(ctx: Context, verbose: bool, warn_skew: bool) -> CliResult<Self> {
        let opts = ClientOptions {
            base_url: ctx.url,
            token: Some(ctx.token),
            pinned_tls_sha256: None,
            user_agent: concat!("rig-cli/", env!("CARGO_PKG_VERSION")).into(),
            verbose,
            warn_skew,
        };
        Ok(Self {
            inner: InnerClient::build(opts)?,
        })
    }

    pub fn base(&self) -> &str {
        self.inner.base()
    }

    pub fn token(&self) -> &str {
        self.inner.token()
    }

    pub fn check_version_skew(&self, server_version: &str) {
        self.inner.check_version_skew(server_version);
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> CliResult<T> {
        Ok(self.inner.get(path).await?)
    }

    pub async fn post<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: Option<&B>,
    ) -> CliResult<T> {
        Ok(self.inner.post(path, body).await?)
    }

    pub async fn post_empty(&self, path: &str) -> CliResult<()> {
        Ok(self.inner.post_empty(path).await?)
    }

    pub async fn patch<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> CliResult<T> {
        Ok(self.inner.patch(path, body).await?)
    }

    pub async fn put<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> CliResult<T> {
        Ok(self.inner.put(path, body).await?)
    }

    pub async fn delete(&self, path: &str) -> CliResult<()> {
        Ok(self.inner.delete(path).await?)
    }

    // --------------------------------------------------------- local uploads

    pub async fn check_blobs(&self, hashes: &[String]) -> CliResult<CheckBlobsResponse> {
        Ok(self.inner.check_blobs(hashes).await?)
    }

    pub async fn upload_blob_zstd(
        &self,
        sha256: &str,
        zstd_bytes: Vec<u8>,
    ) -> CliResult<StoredBlob> {
        Ok(self.inner.upload_blob_zstd(sha256, zstd_bytes).await?)
    }

    pub async fn upload_blob_batch_zstd(
        &self,
        zstd_tar_bytes: Vec<u8>,
    ) -> CliResult<StoredBlobBatch> {
        Ok(self.inner.upload_blob_batch_zstd(zstd_tar_bytes).await?)
    }

    pub async fn deploy_local_image(
        &self,
        app_id: &str,
        req: &LocalImageDeployRequest,
    ) -> CliResult<Deployment> {
        Ok(self.inner.deploy_local_image(app_id, req).await?)
    }

    pub async fn deploy_local_source(
        &self,
        app_id: &str,
        req: &LocalSourceDeployRequest,
    ) -> CliResult<Deployment> {
        Ok(self.inner.deploy_local_source(app_id, req).await?)
    }

    pub async fn deploy_binary(
        &self,
        app_id: &str,
        req: &BinaryDeployRequest,
    ) -> CliResult<Deployment> {
        Ok(self.inner.deploy_binary(app_id, req).await?)
    }

    // ----------------------------------------------------------------- adopt

    pub async fn list_adoptable_containers(&self) -> CliResult<Vec<DiscoveredContainer>> {
        Ok(self.inner.list_adoptable_containers().await?)
    }

    pub async fn preview_adopt(&self, req: &AdoptPreviewRequest) -> CliResult<AdoptPreview> {
        Ok(self.inner.preview_adopt(req).await?)
    }

    pub async fn execute_adopt(
        &self,
        req: &AdoptExecuteRequest,
    ) -> CliResult<AdoptExecuteResult> {
        Ok(self.inner.execute_adopt(req).await?)
    }

    pub async fn download(&self, path: &str, dest: Option<&Path>) -> CliResult<(PathBuf, u64)> {
        Ok(self.inner.download(path, dest).await?)
    }

    pub async fn events(&self, path: &str, on_event: impl FnMut(SseEvent)) -> CliResult<()> {
        Ok(self.inner.events(path, on_event).await?)
    }

    pub async fn events_resumable(
        &self,
        base_path: &str,
        on_event: impl FnMut(SseEvent),
    ) -> CliResult<()> {
        Ok(self.inner.events_resumable(base_path, on_event).await?)
    }

    // --------------------------------------------------------- system & auth

    pub async fn me(&self) -> CliResult<Me> {
        Ok(self.inner.me().await?)
    }

    pub async fn system(&self) -> CliResult<SystemInfo> {
        Ok(self.inner.system().await?)
    }

    pub async fn list_tokens(&self) -> CliResult<Vec<ApiToken>> {
        Ok(self.inner.list_tokens().await?)
    }

    pub async fn create_token(&self, req: &CreateApiTokenRequest) -> CliResult<CreatedApiToken> {
        Ok(self.inner.create_token(req).await?)
    }

    pub async fn delete_token(&self, token_id: &str) -> CliResult<()> {
        Ok(self.inner.delete_token(token_id).await?)
    }

    pub async fn list_devices(&self) -> CliResult<Vec<ApiToken>> {
        Ok(self.inner.list_devices().await?)
    }

    pub async fn create_pairing(&self, req: &CreatePairingRequest) -> CliResult<PairingCode> {
        Ok(self.inner.create_pairing(req).await?)
    }

    pub async fn revoke_device(&self, device_id: &str) -> CliResult<()> {
        Ok(self.inner.revoke_device(device_id).await?)
    }

    pub async fn list_audit(&self) -> CliResult<Vec<AuditEntry>> {
        Ok(self.inner.list_audit().await?)
    }

    // -------------------------------------------------------------- projects

    pub async fn list_projects(&self) -> CliResult<Vec<Project>> {
        Ok(self.inner.list_projects().await?)
    }

    pub async fn create_project(&self, req: &CreateProjectRequest) -> CliResult<Project> {
        Ok(self.inner.create_project(req).await?)
    }

    pub async fn update_project(
        &self,
        project_id: &str,
        req: &UpdateProjectRequest,
    ) -> CliResult<Project> {
        Ok(self.inner.update_project(project_id, req).await?)
    }

    pub async fn delete_project(
        &self,
        project_id: &str,
        databases_action: Option<&str>,
        target_project_id: Option<&str>,
    ) -> CliResult<()> {
        Ok(self
            .inner
            .delete_project(project_id, databases_action, target_project_id)
            .await?)
    }

    // ------------------------------------------------------------------ apps

    pub async fn list_apps(&self, project_id: Option<&str>) -> CliResult<Vec<App>> {
        Ok(self.inner.list_apps(project_id).await?)
    }

    pub async fn resolve_app_ref(&self, app_ref: &str) -> CliResult<App> {
        Ok(self.inner.resolve_app_ref(app_ref).await?)
    }

    pub async fn create_app(&self, req: &CreateAppRequest) -> CliResult<App> {
        Ok(self.inner.create_app(req).await?)
    }

    pub async fn get_app(&self, app_id: &str) -> CliResult<App> {
        Ok(self.inner.get_app(app_id).await?)
    }

    pub async fn update_app(&self, app_id: &str, req: &UpdateAppRequest) -> CliResult<App> {
        Ok(self.inner.update_app(app_id, req).await?)
    }

    pub async fn delete_app(&self, app_id: &str) -> CliResult<()> {
        Ok(self.inner.delete_app(app_id).await?)
    }

    pub async fn deploy_app(&self, app_id: &str) -> CliResult<Deployment> {
        Ok(self.inner.deploy_app(app_id).await?)
    }

    pub async fn start_app(&self, app_id: &str) -> CliResult<Deployment> {
        Ok(self.inner.start_app(app_id).await?)
    }

    pub async fn restart_app(&self, app_id: &str) -> CliResult<Deployment> {
        Ok(self.inner.restart_app(app_id).await?)
    }

    pub async fn stop_app(&self, app_id: &str) -> CliResult<App> {
        Ok(self.inner.stop_app(app_id).await?)
    }

    pub async fn get_webhook(&self, app_id: &str) -> CliResult<AppWebhook> {
        Ok(self.inner.get_webhook(app_id).await?)
    }

    pub async fn rotate_webhook(&self, app_id: &str) -> CliResult<AppWebhook> {
        Ok(self.inner.rotate_webhook(app_id).await?)
    }

    // ----------------------------------------------------------- deployments

    pub async fn list_deployments(&self, app_id: &str) -> CliResult<Vec<Deployment>> {
        Ok(self.inner.list_deployments(app_id).await?)
    }

    pub async fn list_deployments_page(
        &self,
        app_id: &str,
        q: &DeploymentListQuery,
    ) -> CliResult<Vec<Deployment>> {
        Ok(self.inner.list_deployments_page(app_id, q).await?)
    }

    pub async fn get_deployment(&self, deployment_id: &str) -> CliResult<Deployment> {
        Ok(self.inner.get_deployment(deployment_id).await?)
    }

    pub async fn rollback_deployment(&self, deployment_id: &str) -> CliResult<Deployment> {
        Ok(self.inner.rollback_deployment(deployment_id).await?)
    }

    pub async fn cancel_deployment(&self, deployment_id: &str) -> CliResult<Deployment> {
        Ok(self.inner.cancel_deployment(deployment_id).await?)
    }

    // -------------------------------------------------------------- env vars

    pub async fn list_env(&self, app_id: &str) -> CliResult<Vec<EnvVar>> {
        Ok(self.inner.list_env(app_id).await?)
    }

    pub async fn set_env(&self, app_id: &str, req: &SetEnvVarRequest) -> CliResult<EnvVar> {
        Ok(self.inner.set_env(app_id, req).await?)
    }

    pub async fn set_env_bulk(
        &self,
        app_id: &str,
        req: &BulkSetEnvRequest,
    ) -> CliResult<Vec<EnvVar>> {
        Ok(self.inner.set_env_bulk(app_id, req).await?)
    }

    pub async fn reveal_env(&self, app_id: &str, key: &str) -> CliResult<RevealedEnvVar> {
        Ok(self.inner.reveal_env(app_id, key).await?)
    }

    pub async fn delete_env(&self, app_id: &str, key: &str) -> CliResult<()> {
        Ok(self.inner.delete_env(app_id, key).await?)
    }

    // --------------------------------------------------------------- domains

    pub async fn list_domains(&self, app_id: &str) -> CliResult<Vec<Domain>> {
        Ok(self.inner.list_domains(app_id).await?)
    }

    pub async fn add_domain(&self, app_id: &str, req: &AddDomainRequest) -> CliResult<Domain> {
        Ok(self.inner.add_domain(app_id, req).await?)
    }

    pub async fn delete_domain(&self, app_id: &str, domain_id: &str) -> CliResult<()> {
        Ok(self.inner.delete_domain(app_id, domain_id).await?)
    }

    // ---------------------------------------------------- databases & backups

    pub async fn list_databases(&self) -> CliResult<Vec<Database>> {
        Ok(self.inner.list_databases().await?)
    }

    pub async fn list_databases_in(&self, project_id: Option<&str>) -> CliResult<Vec<Database>> {
        Ok(self.inner.list_databases_in(project_id).await?)
    }

    pub async fn list_database_engines(&self) -> CliResult<Vec<DatabaseEngineInfo>> {
        Ok(self.inner.list_database_engines().await?)
    }

    pub async fn create_database(&self, req: &CreateDatabaseRequest) -> CliResult<Database> {
        Ok(self.inner.create_database(req).await?)
    }

    pub async fn get_database(&self, db_id: &str) -> CliResult<Database> {
        Ok(self.inner.get_database(db_id).await?)
    }

    pub async fn start_database(&self, db_id: &str) -> CliResult<Database> {
        Ok(self.inner.start_database(db_id).await?)
    }

    pub async fn stop_database(&self, db_id: &str) -> CliResult<Database> {
        Ok(self.inner.stop_database(db_id).await?)
    }

    pub async fn delete_database(&self, db_id: &str) -> CliResult<()> {
        Ok(self.inner.delete_database(db_id).await?)
    }

    pub async fn reveal_database(&self, db_id: &str) -> CliResult<DatabaseCredentials> {
        Ok(self.inner.reveal_database(db_id).await?)
    }

    pub async fn list_backups(&self, db_id: &str) -> CliResult<Vec<Backup>> {
        Ok(self.inner.list_backups(db_id).await?)
    }

    pub async fn create_backup(&self, db_id: &str) -> CliResult<Backup> {
        Ok(self.inner.create_backup(db_id).await?)
    }

    pub async fn delete_backup(&self, backup_id: &str) -> CliResult<()> {
        Ok(self.inner.delete_backup(backup_id).await?)
    }

    pub async fn restore_backup(&self, backup_id: &str) -> CliResult<()> {
        Ok(self.inner.restore_backup(backup_id).await?)
    }

    pub async fn download_backup(
        &self,
        backup_id: &str,
        dest: Option<&Path>,
    ) -> CliResult<(PathBuf, u64)> {
        Ok(self.inner.download_backup(backup_id, dest).await?)
    }

    // --------------------------------------------------------------- metrics

    pub async fn host_metrics(&self, range: MetricRange) -> CliResult<MetricSeries> {
        Ok(self.inner.host_metrics(range).await?)
    }

    pub async fn app_metrics(
        &self,
        app_id: &str,
        range: MetricRange,
    ) -> CliResult<MetricSeries> {
        Ok(self.inner.app_metrics(app_id, range).await?)
    }

    pub async fn database_metrics(
        &self,
        db_id: &str,
        range: MetricRange,
    ) -> CliResult<MetricSeries> {
        Ok(self.inner.database_metrics(db_id, range).await?)
    }

    // --------------------------------------------------------------- members

    pub async fn list_members(&self) -> CliResult<Vec<Member>> {
        Ok(self.inner.list_members().await?)
    }

    pub async fn add_member(&self, req: &AddMemberRequest) -> CliResult<MemberWithPassword> {
        Ok(self.inner.add_member(req).await?)
    }

    pub async fn update_member(
        &self,
        user_id: &str,
        req: &UpdateMemberRequest,
    ) -> CliResult<Member> {
        Ok(self.inner.update_member(user_id, req).await?)
    }

    pub async fn remove_member(&self, user_id: &str) -> CliResult<()> {
        Ok(self.inner.remove_member(user_id).await?)
    }

    pub async fn reset_member_password(&self, user_id: &str) -> CliResult<MemberWithPassword> {
        Ok(self.inner.reset_member_password(user_id).await?)
    }

    // ---------------------------------------------------------------- alerts

    pub async fn list_alert_channels(&self) -> CliResult<Vec<AlertChannel>> {
        Ok(self.inner.list_alert_channels().await?)
    }

    pub async fn create_alert_channel(
        &self,
        req: &CreateAlertChannelRequest,
    ) -> CliResult<CreatedAlertChannel> {
        Ok(self.inner.create_alert_channel(req).await?)
    }

    pub async fn update_alert_channel(
        &self,
        id: &str,
        req: &UpdateAlertChannelRequest,
    ) -> CliResult<AlertChannel> {
        Ok(self.inner.update_alert_channel(id, req).await?)
    }

    pub async fn delete_alert_channel(&self, id: &str) -> CliResult<()> {
        Ok(self.inner.delete_alert_channel(id).await?)
    }

    pub async fn test_alert_channel(&self, id: &str) -> CliResult<AlertTestResult> {
        Ok(self.inner.test_alert_channel(id).await?)
    }

    // ---------------------------------------------------------------- github

    pub async fn github_status(&self) -> CliResult<GithubStatus> {
        Ok(self.inner.github_status().await?)
    }

    pub async fn github_repos(&self) -> CliResult<Vec<GithubRepo>> {
        Ok(self.inner.github_repos().await?)
    }

    pub async fn github_branches(
        &self,
        owner: &str,
        repo: &str,
    ) -> CliResult<Vec<GithubBranch>> {
        Ok(self.inner.github_branches(owner, repo).await?)
    }

    // --------------------------------------------------------------- streams

    pub async fn app_logs_stream(
        &self,
        app_id: &str,
        q: &AppLogQuery,
        on_event: impl FnMut(SseEvent),
    ) -> CliResult<()> {
        Ok(self.inner.app_logs_stream(app_id, q, on_event).await?)
    }

    pub async fn deployment_logs_stream(
        &self,
        deployment_id: &str,
        on_event: impl FnMut(SseEvent),
    ) -> CliResult<()> {
        Ok(self
            .inner
            .deployment_logs_stream(deployment_id, on_event)
            .await?)
    }

    pub async fn deployment_logs_stream_with_tail(
        &self,
        deployment_id: &str,
        tail: Option<usize>,
        on_event: impl FnMut(SseEvent),
    ) -> CliResult<()> {
        Ok(self
            .inner
            .deployment_logs_stream_with_tail(deployment_id, tail, on_event)
            .await?)
    }

    pub fn terminal_ws_url(&self, app_id: &str, cols: u16, rows: u16) -> CliResult<String> {
        Ok(self.inner.terminal_ws_url(app_id, cols, rows)?)
    }
}
