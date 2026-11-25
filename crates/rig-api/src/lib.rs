//! Wire types shared by the Rig server, CLI and (through OpenAPI) the dashboard.
//!
//! Timestamps are RFC 3339 UTC strings. Ids are UUIDv7 strings.

use serde::{Deserialize, Serialize};

/// Organization role. Ordered: a higher role includes every lower one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Viewer,
    Developer,
    Admin,
    Owner,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Viewer => "viewer",
            Role::Developer => "developer",
            Role::Admin => "admin",
            Role::Owner => "owner",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "viewer" => Role::Viewer,
            "developer" => Role::Developer,
            "admin" => Role::Admin,
            "owner" => Role::Owner,
            _ => return None,
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct ErrorBody {
    pub error: ErrorDetail,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct ErrorDetail {
    /// Stable machine-readable code, e.g. `unauthorized`, `validation`.
    pub code: String,
    /// Human-readable message, safe to show in the UI.
    pub message: String,
    /// Short-lived single-use challenge token returned when `code == "second_factor_required"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub challenge_token: Option<String>,
}

// ---------------------------------------------------------------- setup/auth

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct PublicBranding {
    pub org_name: String,
    pub logo_url: Option<String>,
    pub accent_color: String,
    pub env_label: Option<String>,
    pub env_color: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct SetupStatus {
    /// False until the first owner account has been created.
    pub setup_complete: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Public branding of the primary organization once setup is complete.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branding: Option<PublicBranding>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct SetupRequest {
    /// One-time token printed by the server on first start.
    pub token: String,
    pub name: String,
    pub email: String,
    pub password: String,
    pub org_name: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct LoginRequest {
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub password: String,
    /// The authenticator code or a recovery code, when the account has a
    /// second factor. A login without one answers 401 `second_factor_required`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// Single-use challenge token from step 1 of two-factor sign-in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub challenge_token: Option<String>,
}

/// An active browser session for the signed-in user.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct SessionInfo {
    /// Short identifier (prefix of the hashed session token) for revoking a specific session.
    pub id: String,
    pub user_agent: Option<String>,
    pub ip: Option<String>,
    pub created_at: String,
    pub last_active_at: Option<String>,
    pub expires_at: String,
    pub current: bool,
}

/// Whether the signed-in user has a second factor.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct TotpStatus {
    pub enabled: bool,
    pub enabled_at: Option<String>,
    pub recovery_codes_left: i64,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct PasswordConfirm {
    pub password: String,
}

/// A new secret to add to an authenticator app. It only takes effect once
/// confirmed with a code.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct TotpEnrollment {
    pub secret: String,
    pub otpauth_uri: String,
    /// The URI as an SVG QR code.
    pub qr_svg: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct TotpCode {
    pub code: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct TotpDisable {
    pub password: String,
    pub code: String,
}

/// Shown once; only hashes are kept.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct RecoveryCodes {
    pub codes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct User {
    pub id: String,
    pub email: String,
    pub name: String,
    #[serde(default)]
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct Organization {
    pub id: String,
    pub name: String,
    pub slug: String,
    #[serde(default)]
    pub logo_url: Option<String>,
    #[serde(default = "default_accent_color")]
    pub accent_color: String,
    #[serde(default)]
    pub env_label: Option<String>,
    #[serde(default)]
    pub env_color: Option<String>,
}

fn default_accent_color() -> String {
    "emerald".into()
}

/// Update organization branding and workspace metadata. Omitted fields are kept.
/// Send an empty string for `logo_url`, `env_label`, or `env_color` to clear it.
#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct UpdateOrganizationRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub logo_url: Option<String>,
    #[serde(default)]
    pub accent_color: Option<String>,
    #[serde(default)]
    pub env_label: Option<String>,
    #[serde(default)]
    pub env_color: Option<String>,
}

/// Update the signed-in user's display name or avatar. Omitted fields are kept.
/// Send an empty string for `avatar_url` to reset to initials.
#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct UpdateProfileRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub avatar_url: Option<String>,
}

/// Upload an image (PNG, JPEG, WebP, GIF, or SVG) encoded as base64.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct UploadImageRequest {
    pub content_type: String,
    pub data_base64: String,
}

/// The signed-in principal. `csrf_token` must be echoed in `x-rig-csrf`
/// on every state-changing request made with the session cookie.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct Me {
    pub user: User,
    pub org: Organization,
    pub role: Role,
    pub csrf_token: String,
    /// True when the account has a temporary password that must be changed before continuing.
    #[serde(default)]
    pub must_change_password: bool,
}

// ------------------------------------------------------------------ tokens

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum TokenKind {
    #[default]
    Api,
    Device,
}

impl TokenKind {
    pub fn as_str(self) -> &'static str {
        match self {
            TokenKind::Api => "api",
            TokenKind::Device => "device",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "device" => TokenKind::Device,
            _ => TokenKind::Api,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct ApiToken {
    pub id: String,
    pub name: String,
    /// First characters of the token, for recognising it in a list.
    pub prefix: String,
    pub role: Role,
    #[serde(default)]
    pub kind: TokenKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_ip: Option<String>,
    pub last_used_at: Option<String>,
    pub expires_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct CreateApiTokenRequest {
    pub name: String,
    pub role: Role,
    /// Days until expiry. Omit for a token that does not expire.
    pub expires_in_days: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct CreatedApiToken {
    pub token: ApiToken,
    /// The full secret. Shown once and never retrievable again.
    pub secret: String,
}

// ----------------------------------------------------------- device pairing

#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct CreatePairingRequest {
    /// Role granted to the paired device (cannot exceed the creator's role).
    #[serde(default)]
    pub role: Option<Role>,
    /// Lifetime in seconds (default 300, max 900).
    #[serde(default)]
    pub expires_in_secs: Option<u32>,
}

/// Short-lived, single-use pairing code issued by the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct PairingCode {
    pub id: String,
    /// Human-readable one-time code, e.g. `ABCD-EFGH`.
    pub code: String,
    pub role: Role,
    pub expires_at: String,
    /// SHA-256 hex fingerprint of the server TLS certificate when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tls_fingerprint: Option<String>,
    /// SVG QR code encoding a pairing URI.
    pub qr_svg: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct ExchangePairingRequest {
    pub code: String,
    /// Human-readable device label, e.g. `"xlr8-thinkpad · Rig Desktop"`.
    pub device_name: String,
}

/// Password (+ optional TOTP) sign-in that directly mints a revocable device token.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct DeviceLoginRequest {
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub password: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub challenge_token: Option<String>,
    pub device_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct PairedDevice {
    pub token: ApiToken,
    /// Full bearer token secret. Shown once and stored in the OS keychain.
    pub secret: String,
    pub user: User,
    pub org: Organization,
    pub role: Role,
}

// ------------------------------------------------------ server meta & events

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum ServerMode {
    #[default]
    Full,
    Headless,
}

impl ServerMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ServerMode::Full => "full",
            ServerMode::Headless => "headless",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "full" => ServerMode::Full,
            "headless" => ServerMode::Headless,
            _ => return None,
        })
    }
}

/// Version, mode, and feature capability negotiation returned by `GET /api/v1/meta`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct ServerMeta {
    pub server_version: String,
    pub api_version: u32,
    /// Populated only for authenticated callers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<ServerMode>,
    /// Populated only for authenticated callers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access: Option<Access>,
    /// Feature flags supported by this server build.
    #[serde(default)]
    pub capabilities: Vec<String>,
}

/// One event pushed on `GET /api/v1/events` (SSE).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct ServerEvent {
    pub id: u64,
    /// e.g. `deploy.started`, `deploy.finished`, `deploy.failed`, `app.crashed`, `app.down`, `alert.fired`.
    pub kind: String,
    pub at: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

// ------------------------------------------------------------------- audit

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct AuditEntry {
    pub id: String,
    pub actor: String,
    pub action: String,
    pub target: Option<String>,
    pub detail: Option<String>,
    pub ip: Option<String>,
    pub created_at: String,
}

// ---------------------------------------------------------------- projects

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct Project {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: String,
    pub app_count: u32,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct CreateProjectRequest {
    pub name: String,
    #[serde(default)]
    pub description: String,
}

/// Rename a project or change its description. Omitted fields are kept.
/// The slug stays as it was, so nothing addressing the project breaks.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct UpdateProjectRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// What happens to a project's databases when the project is deleted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum ProjectDatabasesOnDelete {
    /// Move them, with their data and backups, to another project.
    Move,
    /// Delete them with their data and backups.
    Delete,
}

// -------------------------------------------------------------------- apps

/// Where an app's image comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    /// A prebuilt image pulled from a registry.
    Image,
    /// A git repository built from its Dockerfile.
    Git,
    /// Deployed directly from a local folder or binary via `rig up`.
    Local,
}

impl SourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SourceKind::Image => "image",
            SourceKind::Git => "git",
            SourceKind::Local => "local",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "image" => SourceKind::Image,
            "git" => SourceKind::Git,
            "local" => SourceKind::Local,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum AppStatus {
    /// Never deployed.
    Idle,
    Deploying,
    Running,
    Stopped,
    Failed,
}

impl AppStatus {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "idle" => AppStatus::Idle,
            "deploying" => AppStatus::Deploying,
            "running" => AppStatus::Running,
            "stopped" => AppStatus::Stopped,
            "failed" => AppStatus::Failed,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct App {
    pub id: String,
    pub project_id: String,
    pub project_name: String,
    pub name: String,
    pub slug: String,
    pub source_kind: SourceKind,
    pub image: Option<String>,
    pub git_url: Option<String>,
    pub git_branch: Option<String>,
    pub dockerfile_path: String,
    /// Port the app listens on inside its container.
    pub port: u16,
    pub memory_mb: Option<u32>,
    /// CPU limit in thousandths of a core.
    pub cpu_millis: Option<u32>,
    /// HTTP path that must answer before traffic is switched. TCP check when unset.
    pub health_path: Option<String>,
    pub status: AppStatus,
    pub current_deployment_id: Option<String>,
    /// Loopback port of the live container, for reaching the app without a domain.
    pub host_port: Option<u16>,
    pub domains: Vec<String>,
    /// Paths inside the container backed by persistent volumes. Apps with
    /// volumes deploy stop-then-start so two containers never share one.
    pub volumes: Vec<String>,
    /// The latest crash the watcher saw on the live container, if any.
    pub last_crash: Option<AppCrash>,
    /// Git apps: deploy on every push to the branch (GitHub App webhook or polling).
    pub auto_deploy: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct AppCrash {
    pub at: String,
    pub reason: String,
    /// Restarts of the live container so far.
    pub restarts: u32,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct CreateAppRequest {
    pub project_id: String,
    pub name: String,
    pub source_kind: SourceKind,
    /// Required for `image` sources, e.g. `nginx:alpine`.
    pub image: Option<String>,
    /// Required for `git` sources. `https://` or `ssh://` URL.
    pub git_url: Option<String>,
    pub git_branch: Option<String>,
    pub dockerfile_path: Option<String>,
    pub port: u16,
    pub memory_mb: Option<u32>,
    pub cpu_millis: Option<u32>,
    pub health_path: Option<String>,
    /// Absolute paths inside the container to keep across deploys.
    #[serde(default)]
    pub volumes: Vec<String>,
    /// Environment variables set before the first deploy.
    #[serde(default)]
    pub env: Vec<SetEnvVarRequest>,
}

/// Every field is optional; only the ones present are changed. Changes take
/// effect on the next deployment.
#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct UpdateAppRequest {
    pub name: Option<String>,
    pub image: Option<String>,
    pub git_url: Option<String>,
    pub git_branch: Option<String>,
    pub dockerfile_path: Option<String>,
    pub port: Option<u16>,
    /// 0 removes the limit.
    pub memory_mb: Option<u32>,
    /// 0 removes the limit.
    pub cpu_millis: Option<u32>,
    /// An empty string switches back to the TCP check.
    pub health_path: Option<String>,
    /// Git apps only: deploy on every push to the branch.
    pub auto_deploy: Option<bool>,
}

// ------------------------------------------------------------- deployments

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum DeploymentStatus {
    Queued,
    Building,
    Deploying,
    /// Live and receiving traffic.
    Running,
    /// Replaced by a newer deployment. Its image is kept for rollback.
    Superseded,
    Failed,
    Cancelled,
}

impl DeploymentStatus {
    pub fn is_finished(self) -> bool {
        !matches!(self, Self::Queued | Self::Building | Self::Deploying)
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "queued" => DeploymentStatus::Queued,
            "building" => DeploymentStatus::Building,
            "deploying" => DeploymentStatus::Deploying,
            "running" => DeploymentStatus::Running,
            "superseded" => DeploymentStatus::Superseded,
            "failed" => DeploymentStatus::Failed,
            "cancelled" => DeploymentStatus::Cancelled,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct Deployment {
    pub id: String,
    pub app_id: String,
    pub status: DeploymentStatus,
    /// `manual`, `webhook`, `rollback` or `api`.
    pub trigger: String,
    pub image_ref: Option<String>,
    pub commit_sha: Option<String>,
    pub commit_msg: Option<String>,
    pub error: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct LogLine {
    pub seq: i64,
    pub ts: String,
    /// `build`, `deploy`, `stdout` or `stderr`.
    pub stream: String,
    pub line: String,
}

/// Payload of the `end` SSE event emitted when a deployment finishes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct DeploymentEndEvent {
    pub status: String,
    #[serde(default)]
    pub domains: Vec<String>,
}

// ---------------------------------------------------------------- env vars

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct EnvVar {
    pub key: String,
    /// `None` for secrets; use the reveal endpoint to read one.
    pub value: Option<String>,
    pub is_secret: bool,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct SetEnvVarRequest {
    pub key: String,
    pub value: String,
    pub is_secret: bool,
}

/// Atomically create or update multiple environment variables in one transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct BulkSetEnvRequest {
    pub vars: Vec<SetEnvVarRequest>,
    /// When true, remove existing variables that are not in `vars`.
    #[serde(default)]
    pub replace: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct RevealedEnvVar {
    pub key: String,
    pub value: String,
}

// ----------------------------------------------------------------- domains

/// How a domain is served.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum TlsMode {
    /// Public HTTPS with an automatically issued certificate.
    Auto,
    /// HTTPS with a certificate from Rig's own local authority (LAN, tailnet).
    Internal,
    /// Plain HTTP, for use behind another proxy or tunnel.
    Off,
    /// HTTPS with the machine's certificate from Tailscale. Only for the
    /// machine's own `*.ts.net` name.
    Tailscale,
}

impl TlsMode {
    pub fn as_str(self) -> &'static str {
        match self {
            TlsMode::Auto => "auto",
            TlsMode::Internal => "internal",
            TlsMode::Off => "off",
            TlsMode::Tailscale => "tailscale",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "auto" => TlsMode::Auto,
            "internal" => TlsMode::Internal,
            "off" => TlsMode::Off,
            "tailscale" => TlsMode::Tailscale,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct Domain {
    pub id: String,
    pub host: String,
    pub tls_mode: TlsMode,
    pub access: Access,
    /// A listener port of its own, for names that several apps share (a
    /// machine has one Tailscale name). None serves on the standard ports.
    pub port: Option<u16>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct AddDomainRequest {
    pub host: String,
    pub tls_mode: TlsMode,
    #[serde(default)]
    pub access: Access,
    #[serde(default)]
    pub port: Option<u16>,
}

/// Who may reach a domain or the dashboard, judged by the connecting
/// address.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum Access {
    /// Anyone.
    #[default]
    Public,
    /// Private networks only: 10/8, 172.16/12, 192.168/16, link-local,
    /// fc00::/7 and this machine. Includes Tailscale addresses.
    Lan,
    /// Devices on your tailnet (100.64/10, fd7a:115c:a1e0::/48) and this machine.
    Tailscale,
    /// Loopback only (127.0.0.1 / ::1). Reachable on-box or over an SSH tunnel.
    Local,
}

impl Access {
    pub fn as_str(self) -> &'static str {
        match self {
            Access::Public => "public",
            Access::Lan => "lan",
            Access::Tailscale => "tailscale",
            Access::Local => "local",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "public" => Access::Public,
            "lan" => Access::Lan,
            "tailscale" => Access::Tailscale,
            "local" => Access::Local,
            _ => return None,
        })
    }
}

/// Where this machine can be reached, to help pick a domain.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct NetworkInfo {
    /// Public IP address of this server, when detected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_ip: Option<String>,
    /// Private addresses of this machine's interfaces.
    pub lan_addresses: Vec<String>,
    pub tailscale: Option<TailscaleInfo>,
    /// Who may open the dashboard.
    pub dashboard_access: Access,
    /// The address this request came from, as Rig sees it.
    pub your_address: Option<String>,
    /// The hostname the dashboard is served on. Without one, the proxy
    /// serves the dashboard on plain HTTP at any address of this machine.
    pub dashboard_domain: Option<DashboardDomain>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct DashboardDomain {
    pub host: String,
    pub tls_mode: TlsMode,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct DashboardDomainRequest {
    /// The hostname, or null to serve the dashboard on the machine's
    /// addresses over plain HTTP again.
    pub host: Option<String>,
    /// `auto` (public certificate), `internal` or `tailscale`. Defaults to `auto`.
    pub tls_mode: Option<TlsMode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct TailscaleInfo {
    /// The machine's MagicDNS name, e.g. `box.tail1234.ts.net`.
    pub dns_name: Option<String>,
    pub addresses: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct DashboardAccessRequest {
    pub access: Access,
}

// ------------------------------------------------------------------ system

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct SystemInfo {
    pub version: String,
    /// `docker` or `podman`, when the engine answers.
    pub engine: Option<String>,
    pub engine_version: Option<String>,
    /// False when the proxy's admin endpoint cannot be reached; apps still
    /// run and stay reachable on their loopback ports.
    pub proxy_ready: bool,
    pub cpu_percent: f32,
    pub memory_used: u64,
    pub memory_total: u64,
    pub disk_used: u64,
    pub disk_total: u64,
    pub apps_running: u32,
    pub apps_total: u32,
}

// ---------------------------------------------------------------- databases

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum DatabaseEngine {
    Postgres,
    Mysql,
    Mariadb,
    Redis,
    Mongodb,
}

impl DatabaseEngine {
    pub fn as_str(self) -> &'static str {
        match self {
            DatabaseEngine::Postgres => "postgres",
            DatabaseEngine::Mysql => "mysql",
            DatabaseEngine::Mariadb => "mariadb",
            DatabaseEngine::Redis => "redis",
            DatabaseEngine::Mongodb => "mongodb",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "postgres" => DatabaseEngine::Postgres,
            "mysql" => DatabaseEngine::Mysql,
            "mariadb" => DatabaseEngine::Mariadb,
            "redis" => DatabaseEngine::Redis,
            "mongodb" => DatabaseEngine::Mongodb,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum DatabaseStatus {
    /// The image is being pulled or the server is starting.
    Creating,
    Running,
    /// The container is gone; the data volume is kept.
    Stopped,
    Failed,
}

impl DatabaseStatus {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "creating" => DatabaseStatus::Creating,
            "running" => DatabaseStatus::Running,
            "stopped" => DatabaseStatus::Stopped,
            "failed" => DatabaseStatus::Failed,
            _ => return None,
        })
    }
}

/// A managed database. Its password is only returned by the reveal endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct Database {
    pub id: String,
    pub project_id: String,
    pub project_name: String,
    pub name: String,
    pub engine: DatabaseEngine,
    pub version: String,
    pub status: DatabaseStatus,
    /// Why the last start failed.
    pub error: Option<String>,
    /// Hostname apps in the same project use to reach it.
    pub host: String,
    /// Port on that hostname.
    pub port: u16,
    pub username: String,
    pub db_name: String,
    /// Loopback port on the server while running, for tunnels and admin tools.
    pub host_port: Option<u16>,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct CreateDatabaseRequest {
    pub project_id: String,
    pub name: String,
    pub engine: DatabaseEngine,
    /// One of the engine's offered versions. Defaults to the newest.
    #[serde(default)]
    pub version: Option<String>,
}

/// What the dashboard offers when creating a database.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct DatabaseEngineInfo {
    pub engine: DatabaseEngine,
    pub label: String,
    /// Newest first.
    pub versions: Vec<String>,
    pub port: u16,
}

/// Credentials for a database, as returned by the audited reveal endpoint.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct DatabaseCredentials {
    pub password: String,
    /// Connection URL for apps in the same project.
    pub url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum BackupStatus {
    Running,
    Ok,
    Failed,
}

impl BackupStatus {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "running" => BackupStatus::Running,
            "ok" => BackupStatus::Ok,
            "failed" => BackupStatus::Failed,
            _ => return None,
        })
    }
}

/// A dump of one managed database, kept on the server's disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct Backup {
    pub id: String,
    pub database_id: String,
    pub status: BackupStatus,
    pub size_bytes: Option<i64>,
    /// Why the dump failed.
    pub error: Option<String>,
    /// Whether this backup can be restored in place. Redis dumps cannot yet.
    pub restorable: bool,
    pub created_at: String,
    pub finished_at: Option<String>,
}

// ----------------------------------------------------------------- members

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct Member {
    pub user_id: String,
    pub name: String,
    pub email: String,
    #[serde(default)]
    pub avatar_url: Option<String>,
    pub role: Role,
    pub joined_at: String,
    pub last_login_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct AddMemberRequest {
    pub name: String,
    pub email: String,
    pub role: Role,
}

/// A member together with a temporary password, shown once.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct MemberWithPassword {
    pub member: Member,
    /// Hand this to the person; they should change it after signing in.
    pub temporary_password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct UpdateMemberRequest {
    pub role: Role,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

// ----------------------------------------------------------------- metrics

/// How far back a metric series reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum MetricRange {
    #[default]
    #[serde(rename = "1h")]
    Hour,
    #[serde(rename = "6h")]
    SixHours,
    #[serde(rename = "24h")]
    Day,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct MetricPoint {
    /// Start of the interval, in Unix seconds.
    pub ts: i64,
    /// Average CPU use. For the host, percent of all cores; for a container,
    /// percent of one core (so it can exceed 100).
    pub cpu_percent: f64,
    pub memory_bytes: u64,
    /// Average network throughput in bytes per second. Missing until two
    /// samples exist to measure between.
    pub net_rx_bps: Option<f64>,
    pub net_tx_bps: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct MetricSeries {
    pub range: MetricRange,
    /// Seconds each point covers.
    pub step_seconds: u32,
    /// Host memory, or the container's limit. Zero when unlimited or unknown.
    pub memory_total: u64,
    /// Oldest first. Intervals without samples are absent.
    pub points: Vec<MetricPoint>,
}

/// The organization's GitHub App, when one is connected.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GithubStatus {
    pub app: Option<GithubAppInfo>,
    /// Seconds between branch polls for auto-deploy apps; 0 when polling is off.
    pub poll_interval: u64,
    /// Where GitHub sends push events for this organization.
    pub webhook_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GithubAppInfo {
    pub name: String,
    pub slug: String,
    pub html_url: String,
    /// The account or organization that owns the App.
    pub owner: String,
    /// Install the App on repositories here.
    pub install_url: String,
    pub created_at: String,
}

/// Start creating a GitHub App through GitHub's manifest flow.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GithubManifestRequest {
    /// Create the App under this GitHub organization instead of your account.
    #[serde(default)]
    pub organization: Option<String>,
    /// Optional loopback callback URL (e.g. `http://127.0.0.1:<port>/callback`) for native desktop clients.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redirect_url: Option<String>,
}

/// POST `manifest` as a form field to `post_url` from the browser.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GithubManifest {
    pub post_url: String,
    /// The App manifest, as JSON text.
    pub manifest: String,
}

/// What GitHub sends back to the redirect URL after the App is created.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GithubManifestComplete {
    pub code: String,
    pub state: String,
}

/// A repository the GitHub App is installed on.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GithubRepo {
    pub full_name: String,
    pub clone_url: String,
    pub default_branch: String,
    pub private: bool,
}

/// A branch of a repository the GitHub App can read.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GithubBranch {
    pub name: String,
    /// Whether this is the repository's default branch.
    pub default: bool,
    pub protected: bool,
}

/// Outcome of a GitHub App push delivery.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct GithubPushResult {
    /// Deployments queued, one per matching auto-deploy app.
    pub deployment_ids: Vec<String>,
    /// Why nothing was deployed, when nothing was.
    pub skipped: Option<String>,
}

/// Outcome of a push-to-deploy webhook.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct WebhookResult {
    /// The queued deployment, when one was started.
    pub deployment_id: Option<String>,
    /// Why nothing was deployed, for pings and pushes to other branches.
    pub skipped: Option<String>,
}

/// Where to send push-to-deploy webhooks for an app, and the secret that signs them.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct AppWebhook {
    /// Path on this server; prefix it with the dashboard's address.
    pub path: String,
    pub secret: String,
}

/// Something Rig can alert about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum AlertEvent {
    DeployFailed,
    AppCrashed,
    AppDown,
    DiskLow,
    BackupFailed,
}

impl AlertEvent {
    pub const ALL: [AlertEvent; 5] = [
        AlertEvent::DeployFailed,
        AlertEvent::AppCrashed,
        AlertEvent::AppDown,
        AlertEvent::DiskLow,
        AlertEvent::BackupFailed,
    ];
}

/// A URL Rig posts alerts to. The URL itself is never returned, only its host.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct AlertChannel {
    pub id: String,
    pub name: String,
    pub url_host: String,
    pub events: Vec<AlertEvent>,
    pub enabled: bool,
    pub last_sent_at: Option<String>,
    /// Why the most recent delivery failed; empty after a success.
    pub last_error: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct CreateAlertChannelRequest {
    pub name: String,
    pub url: String,
    pub events: Vec<AlertEvent>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct UpdateAlertChannelRequest {
    pub name: Option<String>,
    pub events: Option<Vec<AlertEvent>>,
    pub enabled: Option<bool>,
}

/// A new channel, with the secret that signs its requests. Shown once.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct CreatedAlertChannel {
    pub channel: AlertChannel,
    pub signing_secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct AlertTestResult {
    pub ok: bool,
    pub error: Option<String>,
}

// --------------------------------------------------- local deploys (`rig up`)

/// Check which content-addressed SHA-256 blobs are already cached on the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct CheckBlobsRequest {
    pub hashes: Vec<String>,
}

/// Result of checking the server's content-addressed blob cache.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct CheckBlobsResponse {
    pub present: Vec<String>,
    pub missing: Vec<String>,
}

/// Summary of a stored content-addressed blob.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct StoredBlob {
    pub sha256: String,
    pub size: u64,
}

/// Summary of a batch delta tarball upload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct StoredBlobBatch {
    pub stored: usize,
    pub bytes: u64,
}

/// One file in a local build context manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct ContextFileEntry {
    /// Relative POSIX path inside the project directory (no leading `/` or `..`).
    pub path: String,
    /// Lowercase hex SHA-256 of the file's contents.
    pub sha256: String,
    /// Unix file mode bits (e.g. `0o644` or `0o755`).
    pub mode: u32,
    pub size: u64,
}

/// Deploy an app by materializing a local directory on the server and building it remotely.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct LocalSourceDeployRequest {
    pub files: Vec<ContextFileEntry>,
    /// Optional Dockerfile/Containerfile path relative to the context root.
    #[serde(default)]
    pub dockerfile: Option<String>,
    /// Optional short description recorded on the deployment (e.g. folder name).
    #[serde(default)]
    pub label: Option<String>,
}

/// One entry in a locally built OCI/Docker image archive, backed by the server blob cache.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct ArchiveBlobEntry {
    /// Tar entry path inside the saved image archive (e.g. `manifest.json`, `blobs/sha256/...`).
    pub archive_path: String,
    /// Lowercase hex SHA-256 of the entry bytes stored in the blob cache.
    pub sha256: String,
    /// Unix mode bits for the tar entry.
    #[serde(default = "default_archive_mode")]
    pub mode: u32,
}

fn default_archive_mode() -> u32 {
    0o644
}

/// Deploy an app using an OCI/Docker image built locally on the developer's machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct LocalImageDeployRequest {
    /// Image reference / tag inside the saved archive (e.g. `rig/my-app:up-1a2b3c4d`).
    pub image_ref: String,
    /// Ordered entries that reconstruct the Docker/OCI image tarball from cached blobs.
    pub entries: Vec<ArchiveBlobEntry>,
    /// Optional label recorded on the deployment.
    #[serde(default)]
    pub label: Option<String>,
}

/// Deploy a pre-compiled Linux binary directly without a Dockerfile.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct BinaryDeployRequest {
    /// Lowercase hex SHA-256 of the uploaded ELF binary in the blob cache.
    pub sha256: String,
    /// Original filename of the binary (for display/audit).
    pub filename: String,
    /// Optional default command-line arguments passed to the binary.
    #[serde(default)]
    pub args: Vec<String>,
    /// Optional base image override (e.g. `gcr.io/distroless/cc-debian12:nonroot` or `debian:bookworm-slim`).
    /// When omitted, static binaries use `scratch` and dynamically linked binaries use `gcr.io/distroless/cc-debian12:nonroot`.
    #[serde(default)]
    pub base_image: Option<String>,
    /// Optional port override for the app container.
    #[serde(default)]
    pub port: Option<u16>,
}

// -------------------------------------------- adopt existing containers (`rig adopt`)

/// A port binding discovered on an external container.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct DiscoveredPort {
    pub container_port: u16,
    pub protocol: String,
    pub host_ip: Option<String>,
    pub host_port: Option<u16>,
}

/// A bind mount or named volume discovered on an external container.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct DiscoveredMount {
    /// `"bind"` (host path) or `"volume"` (engine named volume).
    pub kind: String,
    /// Exact host path (for `"bind"`) or volume name (for `"volume"`).
    pub source: String,
    /// Mount path inside the container.
    pub destination: String,
    pub read_only: bool,
}

/// An environment variable discovered on an external container.
/// Suspected secrets never include their plaintext value in discovery responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct DiscoveredEnvVar {
    pub key: String,
    /// Plaintext preview for non-secret values; always `None` when `is_secret_guess` is true.
    pub preview_value: Option<String>,
    pub is_secret_guess: bool,
    pub secret_reason: Option<String>,
}

/// A reverse proxy domain rule extracted from container labels or environment variables.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct DiscoveredDomain {
    pub host: String,
    pub tls_mode: TlsMode,
    /// Where the rule came from: `"traefik"`, `"caddy"`, or `"virtual_host"`.
    pub source: String,
    pub service_port: Option<u16>,
}

/// An unmanaged Docker/Podman container discovered on the server and eligible for adoption.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct DiscoveredContainer {
    pub id: String,
    pub name: String,
    pub state: String,
    pub image: String,
    /// Exact immutable repo digest (`repo@sha256:...`), when available.
    pub image_digest: Option<String>,
    /// Local engine content ID (`sha256:...`).
    pub image_id: Option<String>,
    /// `"standalone"`, `"compose"`, `"coolify"`, or `"dokploy"`.
    pub origin: String,
    /// Compose project or Coolify/Dokploy stack name, when part of a stack.
    pub stack: Option<String>,
    /// Compose service name, when present.
    pub service: Option<String>,
    pub ports: Vec<DiscoveredPort>,
    /// Best-guess primary HTTP container port.
    pub detected_port: Option<u16>,
    pub mounts: Vec<DiscoveredMount>,
    pub networks: Vec<String>,
    pub env: Vec<DiscoveredEnvVar>,
    pub domains: Vec<DiscoveredDomain>,
    pub restart_policy: Option<String>,
    pub memory_mb: Option<u32>,
    pub cpu_millis: Option<u32>,
    pub command: Option<Vec<String>>,
    pub entrypoint: Option<Vec<String>>,
    pub health_path: Option<String>,
    pub created_at: Option<String>,
}

/// Request a dry-run preview of adopting an external container.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct AdoptPreviewRequest {
    pub container_id: String,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub app_name: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub health_path: Option<String>,
    /// Optional explicit domains to attach instead of the auto-detected ones.
    #[serde(default)]
    pub domains: Option<Vec<AddDomainRequest>>,
    /// Force these env keys to be encrypted as secrets in Rig's vault.
    #[serde(default)]
    pub secret_keys: Option<Vec<String>>,
    /// Force these env keys to be stored as non-secret (still encrypted at rest, shown unmasked).
    #[serde(default)]
    pub plain_keys: Option<Vec<String>>,
}

/// One environment variable's planned vault classification during adoption.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct AdoptEnvPreview {
    pub key: String,
    pub is_secret: bool,
    pub secret_reason: Option<String>,
}

/// Dry-run preview of adopting an external container into Rig.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct AdoptPreview {
    pub container: DiscoveredContainer,
    pub app_name: String,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub image: String,
    pub port: u16,
    pub health_path: Option<String>,
    pub memory_mb: Option<u32>,
    pub cpu_millis: Option<u32>,
    pub mounts: Vec<DiscoveredMount>,
    pub domains: Vec<AddDomainRequest>,
    pub env_keys: Vec<AdoptEnvPreview>,
    /// Preview of the generated `rig.toml`.
    pub rig_toml: String,
    /// Preview of the generated Podman Quadlet `.container` unit.
    pub quadlet_container: String,
    /// Preview of the generated Podman Quadlet `.network` unit.
    pub quadlet_network: String,
    /// `"zero-downtime"` for stateless containers, `"stop-start-with-rollback"` when writable mounts exist.
    pub cutover_strategy: String,
    pub warnings: Vec<String>,
}

/// Execute adoption of an external container into Rig with safe cutover and automatic rollback.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct AdoptExecuteRequest {
    pub container_id: String,
    pub project_id: String,
    #[serde(default)]
    pub app_name: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub health_path: Option<String>,
    #[serde(default)]
    pub domains: Option<Vec<AddDomainRequest>>,
    #[serde(default)]
    pub secret_keys: Option<Vec<String>>,
    #[serde(default)]
    pub plain_keys: Option<Vec<String>>,
    /// Stop the old container once the adopted container passes health checks and proxy cutover (default true).
    #[serde(default = "default_true")]
    pub stop_old: bool,
    /// Also remove the old container after stopping it (default false: keeps it stopped as a safety net).
    #[serde(default)]
    pub remove_old: bool,
}

fn default_true() -> bool {
    true
}

/// Result of a completed container adoption.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct AdoptExecuteResult {
    pub app: App,
    pub deployment: Deployment,
    pub old_container_id: String,
    pub old_container_stopped: bool,
    pub old_container_removed: bool,
    pub domains_registered: Vec<String>,
    pub volumes_preserved: Vec<DiscoveredMount>,
    pub secrets_encrypted: usize,
    pub rig_toml: String,
    pub quadlet_container: String,
    pub quadlet_network: String,
}
