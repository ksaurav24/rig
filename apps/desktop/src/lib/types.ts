export type ServerHealth = 'connecting' | 'online' | 'deploying' | 'degraded' | 'offline'
export type ServerMode = 'full' | 'headless'
export type AccessMode = 'public' | 'lan' | 'tailscale' | 'local'
export type Role = 'viewer' | 'developer' | 'admin' | 'owner'
export type SourceKind = 'image' | 'git' | 'local'
export type DatabaseEngine = 'postgres' | 'mysql' | 'mariadb' | 'redis' | 'mongodb'
export type AlertEvent = 'deploy_failed' | 'app_crashed' | 'app_down' | 'disk_low' | 'backup_failed'

export interface ServerMeta {
  server_version: string
  api_version: number
  mode?: ServerMode
  access?: AccessMode
  capabilities: string[]
}

export interface SystemInfo {
  version: string
  engine?: string | null
  engine_version?: string | null
  proxy_ready: boolean
  cpu_percent: number
  memory_used: number
  memory_total: number
  disk_used: number
  disk_total: number
  apps_running: number
  apps_total: number
}

export interface User {
  id: string
  email: string
  name: string
  avatar_url?: string | null
}

export interface Organization {
  id: string
  name: string
  slug: string
  logo_url?: string | null
  accent_color: string
  env_label?: string | null
  env_color?: string | null
}

export interface Me {
  user: User
  org: Organization
  role: Role
  csrf_token: string
  must_change_password: boolean
}

export interface Project {
  id: string
  name: string
  slug: string
  description: string
  app_count: number
  created_at: string
}

export interface AppCrash {
  at: string
  reason: string
  restarts: number
}

export interface App {
  id: string
  project_id: string
  project_name: string
  name: string
  slug: string
  source_kind: SourceKind
  image?: string | null
  git_url?: string | null
  git_branch?: string | null
  dockerfile_path: string
  port: number
  memory_mb?: number | null
  cpu_millis?: number | null
  health_path?: string | null
  status: string
  current_deployment_id?: string | null
  host_port?: number | null
  domains: string[]
  volumes: string[]
  last_crash?: AppCrash | null
  auto_deploy: boolean
  created_at: string
  updated_at: string
}

export interface CreateAppRequest {
  project_id: string
  name: string
  source_kind: SourceKind
  image?: string | null
  git_url?: string | null
  git_branch?: string | null
  dockerfile_path?: string | null
  port: number
  memory_mb?: number | null
  cpu_millis?: number | null
  health_path?: string | null
  volumes?: string[]
  env?: SetEnvVarRequest[]
}

export interface UpdateAppRequest {
  name?: string
  image?: string
  git_url?: string
  git_branch?: string
  dockerfile_path?: string
  port?: number
  memory_mb?: number
  cpu_millis?: number
  health_path?: string
  auto_deploy?: boolean
  volumes?: string[]
}

export interface AppWebhook {
  path: string
  secret: string
}

export type MetricRange = '1h' | '6h' | '24h'

export interface MetricPoint {
  ts: number
  cpu_percent: number
  memory_bytes: number
  net_rx_bps?: number | null
  net_tx_bps?: number | null
}

export interface MetricSeries {
  range: MetricRange
  step_seconds: number
  memory_total: number
  points: MetricPoint[]
}

export interface UploadImageRequest {
  content_type: string
  data_base64: string
}

export interface UpdateProfileRequest {
  name?: string
  avatar_url?: string
}

export interface ChangePasswordRequest {
  current_password: string
  new_password: string
}

export interface TotpStatus {
  enabled: boolean
  enabled_at?: string | null
  recovery_codes_left: number
}

export interface TotpEnrollment {
  secret: string
  otpauth_uri: string
  qr_svg: string
}

export interface RecoveryCodes {
  codes: string[]
}

export interface BackupReminder {
  enabled: boolean
  interval_days: number
  last_snapshot_at?: string | null
  last_snapshot_path?: string | null
  last_snapshot_bytes?: number | null
  snoozed_until?: string | null
  last_notified_at?: string | null
}

export interface DownloadedFile {
  path: string
  bytes: number
}

export interface SnapshotExportRequest {
  include_database_dumps: boolean
  include_alerts: boolean
  include_github: boolean
  passphrase?: string | null
}

export interface SnapshotExportResult {
  path: string
  bytes: number
  encrypted: boolean
  reminder: BackupReminder
}

export type SnapshotRestoreMode = 'merge' | 'replace'

export interface SnapshotRestoreOptions {
  mode: SnapshotRestoreMode
  restore_branding: boolean
  restore_alerts: boolean
  restore_github: boolean
  start_databases: boolean
  deploy_running_apps: boolean
}

export interface SnapshotProjectPreview {
  name: string
  slug: string
  description: string
  exists_on_target: boolean
}

export interface SnapshotAppPreview {
  project_slug: string
  name: string
  slug: string
  source_kind: SourceKind
  image?: string | null
  git_url?: string | null
  port: number
  was_running: boolean
  env_count: number
  domain_count: number
  volume_count: number
  exists_on_target: boolean
}

export interface SnapshotDatabasePreview {
  project_slug: string
  name: string
  slug: string
  engine: DatabaseEngine
  version: string
  was_running: boolean
  has_dump: boolean
  dump_size_bytes?: number | null
  dump_error?: string | null
  exists_on_target: boolean
}

export interface SnapshotInspectResult {
  format_version: number
  encrypted: boolean
  created_at: string
  rig_version: string
  source_org_name: string
  source_org_slug: string
  has_logo: boolean
  has_github_app: boolean
  media_count: number
  alert_channel_count: number
  projects: SnapshotProjectPreview[]
  apps: SnapshotAppPreview[]
  databases: SnapshotDatabasePreview[]
  warnings: string[]
}

export interface SnapshotRestoreReport {
  mode: SnapshotRestoreMode
  branding_restored: boolean
  media_restored: number
  projects_created: number
  projects_skipped: number
  apps_created: number
  apps_skipped: number
  apps_deployments_queued: number
  env_vars_restored: number
  domains_restored: number
  volumes_restored: number
  databases_created: number
  databases_skipped: number
  databases_provisioning: number
  database_dumps_scheduled: number
  alerts_created: number
  github_restored: boolean
  warnings: string[]
}

export interface Deployment {
  id: string
  app_id: string
  status: string
  trigger: string
  image_ref?: string | null
  commit_sha?: string | null
  commit_msg?: string | null
  error?: string | null
  started_at?: string | null
  finished_at?: string | null
  created_at: string
}

export interface EnvVar {
  key: string
  value?: string | null
  is_secret: boolean
  updated_at: string
}

export interface SetEnvVarRequest {
  key: string
  value: string
  is_secret: boolean
}

export interface RevealedEnvVar {
  key: string
  value: string
}

export interface Domain {
  id: string
  host: string
  tls_mode: string
  access: AccessMode
  port?: number | null
  created_at: string
}

export interface Database {
  id: string
  project_id: string
  project_name: string
  name: string
  engine: DatabaseEngine
  version: string
  status: string
  error?: string | null
  host: string
  port: number
  username: string
  db_name: string
  host_port?: number | null
  created_at: string
}

export interface DatabaseEngineInfo {
  engine: DatabaseEngine
  label: string
  versions: string[]
  port: number
}

export interface CreateDatabaseRequest {
  project_id: string
  name: string
  engine: DatabaseEngine
  version?: string | null
}

export interface DatabaseCredentials {
  password: string
  url: string
}

export interface Backup {
  id: string
  database_id: string
  status: string
  size_bytes?: number | null
  error?: string | null
  restorable: boolean
  created_at: string
  finished_at?: string | null
}

export interface ApiToken {
  id: string
  name: string
  prefix: string
  role: string
  kind?: 'api' | 'device'
  last_ip?: string | null
  last_used_at?: string | null
  expires_at?: string | null
  created_at: string
}

export interface CreateApiTokenRequest {
  name: string
  role: Role
  expires_in_days?: number | null
}

export interface CreatedApiToken {
  token: ApiToken
  secret: string
}

export interface PairingCode {
  id: string
  code: string
  role: string
  expires_at: string
  tls_fingerprint?: string | null
  qr_svg: string
}

export interface Member {
  user_id: string
  name: string
  email: string
  avatar_url?: string | null
  role: Role
  joined_at: string
  last_login_at?: string | null
}

export interface MemberWithPassword {
  member: Member
  temporary_password: string
}

export interface AlertChannel {
  id: string
  name: string
  url_host: string
  events: AlertEvent[]
  enabled: boolean
  last_sent_at?: string | null
  last_error?: string | null
  created_at: string
}

export interface AuditEntry {
  id: string
  actor: string
  action: string
  target?: string | null
  detail?: string | null
  ip?: string | null
  created_at: string
}

export interface NetworkInfo {
  public_ip?: string | null
  lan_addresses: string[]
  tailscale?: {
    dns_name?: string | null
    addresses: string[]
  } | null
  dashboard_access: AccessMode
  your_address?: string | null
  dashboard_domain?: {
    host: string
    tls_mode: string
  } | null
}

export interface GithubAppInfo {
  name: string
  slug: string
  html_url: string
  owner: string
  install_url: string
  created_at: string
}

export interface GithubStatus {
  app?: GithubAppInfo | null
  poll_interval: number
  webhook_url: string
}

export interface GithubRepo {
  full_name: string
  clone_url: string
  default_branch: string
  private: boolean
}

export interface GithubBranch {
  name: string
  default: boolean
  protected: boolean
}

export interface ServerSettingsSnapshot {
  me?: Me | null
  network?: NetworkInfo | null
  devices: ApiToken[]
  tokens: ApiToken[]
  members: Member[]
  alerts: AlertChannel[]
  audit: AuditEntry[]
  github?: GithubStatus | null
}

export interface ServerEvent {
  id: number
  kind: string
  at: string
  title: string
  detail?: string | null
  app_id?: string | null
  deployment_id?: string | null
  status?: string | null
}

export interface CachedServerSnapshot {
  system?: SystemInfo | null
  projects: Project[]
  apps: App[]
  databases: Database[]
  cached_at?: string | null
}

export interface ServerView {
  id: string
  name: string
  url: string
  accent: string
  pinned_tls_sha256?: string | null
  user_email?: string | null
  org_name?: string | null
  meta?: ServerMeta | null
  health: ServerHealth
  health_error?: string | null
  cache: CachedServerSnapshot
  backup_reminder: BackupReminder
  created_at: string
}

export interface ServersStatePayload {
  active_server_id?: string | null
  servers: ServerView[]
}

export interface TlsProbe {
  is_https: boolean
  fingerprint_sha256?: string | null
  trusted_by_roots: boolean
}

export interface ProbeServerResult {
  normalized_url: string
  tls: TlsProbe
  meta?: ServerMeta | null
  setup?: { setup_complete: boolean } | null
  needs_tls_pin_confirmation: boolean
  api_compatible: boolean
  compatibility_note?: string | null
}

export interface ServerSnapshot {
  server_id: string
  from_cache: boolean
  system?: SystemInfo | null
  projects: Project[]
  apps: App[]
  databases: Database[]
  recent_events: ServerEvent[]
  meta?: ServerMeta | null
  fetch_ms: number
}

export interface AppDetailSnapshot {
  app: App
  deployments: Deployment[]
  env: EnvVar[]
  domains: Domain[]
}

export type DeviceLoginResult =
  | { status: 'paired'; server: ServerView }
  | { status: 'second_factor_required'; challenge_token?: string | null; message: string }

export interface AnsiSpan {
  text: string
  fg?: number
  bold?: boolean
  dim?: boolean
}

export interface ParsedLogLine {
  seq: number
  ts: string
  stream: string
  plain: string
  spans?: AnsiSpan[]
}

export type LogChannelFrame =
  | {
      type: 'batch'
      stream_id: string
      lines: ParsedLogLine[]
      total_buffered: number
      dropped_total: number
    }
  | {
      type: 'status'
      stream_id: string
      state: string
      message?: string | null
    }
  | {
      type: 'bench_summary'
      stream_id: string
      lines_generated: number
      elapsed_ms: number
      actual_lines_per_sec: number
    }

export interface LogQueryResult {
  total_buffered: number
  matched_count: number
  offset: number
  lines: ParsedLogLine[]
}

export type EnvDiffKind = 'added' | 'changed' | 'secret_overwrite' | 'unchanged'

export interface EnvDiffItem {
  key: string
  kind: EnvDiffKind
  current_value?: string | null
  current_is_secret: boolean
  incoming_preview: string
  incoming_is_secret: boolean
  incoming_value: string
}

export interface EnvDiffPreview {
  items: EnvDiffItem[]
  added: number
  changed: number
  secret_overwrites: number
  unchanged: number
  skipped_lines: number
}

export interface LocalGitStatus {
  path: string
  is_git_repo: boolean
  branch?: string | null
  head_sha?: string | null
  head_subject?: string | null
  remote_url?: string | null
  dirty_files: number
  unpushed_commits: number
  has_dockerfile: boolean
  stacks: string[]
}

export interface DiscoveredRepo {
  name: string
  path: string
  branch?: string | null
  head_sha?: string | null
  head_subject?: string | null
  remote_url?: string | null
  dirty_files: number
  unpushed_commits: number
  has_dockerfile: boolean
  stacks: string[]
  modified_epoch: number
}

export type TerminalFrame =
  | { type: 'output'; data: number[] }
  | { type: 'state'; state: string; message?: string | null }

export interface PerfDiagnostics {
  uptime_ms: number
  server_count: number
  log_buffers: {
    stream_id: string
    buffered_lines: number
    capacity: number
    dropped_lines: number
  }[]
  client_version: string
}

export interface CliInfo {
  installed: boolean
  binary_path?: string | null
  version?: string | null
  config_path: string
  config_exists: boolean
  active_context?: string | null
  contexts_count: number
}

export interface DesktopAppInfo {
  app_version: string
  client_version: string
  os: string
  arch: string
  registry_path: string
  repository_url: string
  cli: CliInfo
}

export interface UpdateCheckResult {
  current_version: string
  latest_version?: string | null
  update_available: boolean
  release_url: string
  published_at?: string | null
  notes?: string | null
  checked_at: string
  error?: string | null
}
