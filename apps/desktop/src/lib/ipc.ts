/**
 * Single typed IPC adapter between the React UI and the Rust Tauri core (§5.1).
 *
 * The webview NEVER calls fetch() directly against Rig servers and NEVER sees
 * bearer tokens. All networking, TLS pinning, keychain access, and stream
 * buffering happen in Rust.
 */
import { Channel, invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { openUrl } from '@tauri-apps/plugin-opener'
import type {
  AlertEvent,
  ApiToken,
  App,
  AppDetailSnapshot,
  Backup,
  CreateApiTokenRequest,
  CreateAppRequest,
  CreateDatabaseRequest,
  CreatedApiToken,
  Database,
  DatabaseCredentials,
  DatabaseEngineInfo,
  Deployment,
  DesktopAppInfo,
  DeviceLoginResult,
  DiscoveredRepo,
  Domain,
  EnvDiffItem,
  EnvDiffPreview,
  EnvVar,
  GithubBranch,
  GithubRepo,
  GithubStatus,
  LocalGitStatus,
  LogChannelFrame,
  LogQueryResult,
  Member,
  MemberWithPassword,
  Organization,
  PairingCode,
  PerfDiagnostics,
  ProbeServerResult,
  Project,
  RevealedEnvVar,
  Role,
  ServerEvent,
  ServerHealth,
  ServerSettingsSnapshot,
  ServerSnapshot,
  ServersStatePayload,
  ServerView,
  SetEnvVarRequest,
  TerminalFrame,
  UpdateAppRequest,
  UpdateCheckResult,
} from './types'

export const desktopApi = {
  // ---- Server Registry ----
  listServers: () => invoke<ServersStatePayload>('list_servers'),
  probeServer: (url: string, acceptFingerprint?: string) =>
    invoke<ProbeServerResult>('probe_server', {
      url,
      acceptFingerprint: acceptFingerprint ?? null,
    }),
  initServerSetup: (args: {
    url: string
    pinnedTlsSha256?: string | null
    serverName?: string
    setupToken: string
    orgName: string
    name: string
    email: string
    password: string
  }) => invoke<ServerView>('init_server_setup', args),
  pairServer: (args: {
    url: string
    code: string
    serverName?: string
    pinnedTlsSha256?: string | null
  }) => invoke<ServerView>('pair_server', args),
  loginServer: (args: {
    url: string
    email: string
    password: string
    totpCode?: string
    challengeToken?: string
    serverName?: string
    pinnedTlsSha256?: string | null
  }) => invoke<DeviceLoginResult>('login_server', args),
  selectServer: (serverId: string) => invoke<boolean>('select_server', { serverId }),
  removeServer: (serverId: string) => invoke<boolean>('remove_server', { serverId }),
  reorderServers: (orderedIds: string[]) => invoke<void>('reorder_servers', { orderedIds }),
  updateServer: (serverId: string, args: { name?: string; accent?: string }) =>
    invoke<ServerView>('update_server', {
      serverId,
      name: args.name ?? null,
      accent: args.accent ?? null,
    }),

  // ---- Projects & Apps ----
  getServerSnapshot: (serverId: string) =>
    invoke<ServerSnapshot>('get_server_snapshot', { serverId }),
  getAppDetail: (serverId: string, appId: string) =>
    invoke<AppDetailSnapshot>('get_app_detail', { serverId, appId }),
  createProject: (serverId: string, name: string, description?: string) =>
    invoke<Project>('create_project', { serverId, name, description }),
  updateProject: (serverId: string, projectId: string, name?: string, description?: string) =>
    invoke<Project>('update_project', { serverId, projectId, name, description }),
  deleteProject: (serverId: string, projectId: string) =>
    invoke<void>('delete_project', { serverId, projectId }),
  createApp: (serverId: string, req: CreateAppRequest) =>
    invoke<App>('create_app', { serverId, req }),
  updateApp: (serverId: string, appId: string, req: UpdateAppRequest) =>
    invoke<App>('update_app', { serverId, appId, req }),
  deleteApp: (serverId: string, appId: string) =>
    invoke<void>('delete_app', { serverId, appId }),
  appAction: (serverId: string, appId: string, action: 'deploy' | 'start' | 'restart' | 'stop') =>
    invoke<Deployment | App>('app_action', { serverId, appId, action }),
  rollbackDeployment: (serverId: string, deploymentId: string) =>
    invoke<Deployment>('rollback_deployment', { serverId, deploymentId }),
  cancelDeployment: (serverId: string, deploymentId: string) =>
    invoke<Deployment>('cancel_deployment', { serverId, deploymentId }),
  addDomain: (
    serverId: string,
    appId: string,
    req: { host: string; tls_mode: string; access: string; port?: number | null },
  ) => invoke<Domain>('add_domain', { serverId, appId, req }),
  deleteDomain: (serverId: string, appId: string, domainId: string) =>
    invoke<void>('delete_domain', { serverId, appId, domainId }),

  // ---- Databases & Backups ----
  listDatabaseEngines: (serverId: string) =>
    invoke<DatabaseEngineInfo[]>('list_database_engines', { serverId }),
  createDatabase: (serverId: string, req: CreateDatabaseRequest) =>
    invoke<Database>('create_database', { serverId, req }),
  deleteDatabase: (serverId: string, dbId: string) =>
    invoke<void>('delete_database', { serverId, dbId }),
  databaseAction: (serverId: string, dbId: string, action: 'start' | 'stop') =>
    invoke<Database>('database_action', { serverId, dbId, action }),
  revealDatabase: (serverId: string, dbId: string) =>
    invoke<DatabaseCredentials>('reveal_database', { serverId, dbId }),
  listBackups: (serverId: string, dbId: string) =>
    invoke<Backup[]>('list_backups', { serverId, dbId }),
  createBackup: (serverId: string, dbId: string) =>
    invoke<Backup>('create_backup', { serverId, dbId }),
  restoreBackup: (serverId: string, backupId: string) =>
    invoke<void>('restore_backup', { serverId, backupId }),
  deleteBackup: (serverId: string, backupId: string) =>
    invoke<void>('delete_backup', { serverId, backupId }),

  // ---- Server Settings, Team, Alerts, Devices & Audit ----
  getServerSettings: (serverId: string) =>
    invoke<ServerSettingsSnapshot>('get_server_settings', { serverId }),
  updateOrganization: (
    serverId: string,
    req: { name?: string; accent_color?: string; env_label?: string; env_color?: string },
  ) => invoke<Organization>('update_organization', { serverId, req }),
  addMember: (serverId: string, req: { name: string; email: string; role: Role }) =>
    invoke<MemberWithPassword>('add_member', { serverId, req }),
  updateMember: (serverId: string, userId: string, req: { role: Role }) =>
    invoke<Member>('update_member', { serverId, userId, req }),
  removeMember: (serverId: string, userId: string) =>
    invoke<void>('remove_member', { serverId, userId }),
  resetMemberPassword: (serverId: string, userId: string) =>
    invoke<MemberWithPassword>('reset_member_password', { serverId, userId }),
  createAlertChannel: (
    serverId: string,
    req: { name: string; url: string; events: AlertEvent[] },
  ) => invoke<{ channel: unknown; signing_secret: string }>('create_alert_channel', { serverId, req }),
  testAlertChannel: (serverId: string, id: string) =>
    invoke<{ ok: boolean; error?: string | null }>('test_alert_channel', { serverId, id }),
  deleteAlertChannel: (serverId: string, id: string) =>
    invoke<void>('delete_alert_channel', { serverId, id }),
  createToken: (serverId: string, req: CreateApiTokenRequest) =>
    invoke<CreatedApiToken>('create_token', { serverId, req }),
  revokeToken: (serverId: string, tokenId: string) =>
    invoke<void>('revoke_token', { serverId, tokenId }),
  getGithubStatus: (serverId: string) =>
    invoke<GithubStatus>('get_github_status', { serverId }),
  listGithubRepos: (serverId: string) =>
    invoke<GithubRepo[]>('list_github_repos', { serverId }),
  listGithubBranches: (serverId: string, owner: string, repo: string) =>
    invoke<GithubBranch[]>('list_github_branches', { serverId, owner, repo }),
  connectGithubApp: (serverId: string, organization?: string) =>
    invoke<GithubStatus>('connect_github_app', {
      serverId,
      organization: organization?.trim() || null,
    }),
  cancelGithubConnect: () => invoke<void>('cancel_github_connect'),
  disconnectGithub: (serverId: string) =>
    invoke<void>('disconnect_github', { serverId }),
  listDevices: (serverId: string) => invoke<ApiToken[]>('list_devices', { serverId }),
  createPairingCode: (serverId: string) =>
    invoke<PairingCode>('create_pairing_code', { serverId }),
  revokeDevice: (serverId: string, deviceId: string) =>
    invoke<void>('revoke_device', { serverId, deviceId }),

  // ---- Rust High-Perf Log Engine ----
  startAppLogs: (
    serverId: string,
    appId: string,
    streamId: string,
    onFrame: (frame: LogChannelFrame) => void,
    tail = 500,
  ) => {
    const channel = new Channel<LogChannelFrame>()
    channel.onmessage = onFrame
    return invoke<void>('start_app_logs', {
      serverId,
      appId,
      streamId,
      tail,
      onFrame: channel,
    })
  },
  startDeployLogs: (
    serverId: string,
    deploymentId: string,
    streamId: string,
    onFrame: (frame: LogChannelFrame) => void,
  ) => {
    const channel = new Channel<LogChannelFrame>()
    channel.onmessage = onFrame
    return invoke<void>('start_deploy_logs', {
      serverId,
      deploymentId,
      streamId,
      onFrame: channel,
    })
  },
  startLogBenchmark: (
    streamId: string,
    linesPerSec: number,
    durationSecs: number,
    onFrame: (frame: LogChannelFrame) => void,
  ) => {
    const channel = new Channel<LogChannelFrame>()
    channel.onmessage = onFrame
    return invoke<void>('start_log_benchmark', {
      streamId,
      linesPerSec,
      durationSecs,
      onFrame: channel,
    })
  },
  stopLogStream: (streamId: string) => invoke<void>('stop_log_stream', { streamId }),
  queryLogBuffer: (args: {
    streamId: string
    filter?: string
    streamFilter?: string
    offset: number
    limit: number
  }) => invoke<LogQueryResult>('query_log_buffer', args),
  getPerfDiagnostics: () => invoke<PerfDiagnostics>('get_perf_diagnostics'),
  getAppInfo: () => invoke<DesktopAppInfo>('get_app_info'),
  checkDesktopUpdates: () => invoke<UpdateCheckResult>('check_desktop_updates'),
  openExternalUrl: async (url: string) => {
    try {
      await openUrl(url)
    } catch {
      window.open(url, '_blank', 'noopener,noreferrer')
    }
  },

  // ---- Env Vars (Full CRUD + Local .env Diff) ----
  setEnvVar: (serverId: string, appId: string, req: SetEnvVarRequest) =>
    invoke<EnvVar>('set_env_var', { serverId, appId, req }),
  revealEnvVar: (serverId: string, appId: string, key: string) =>
    invoke<RevealedEnvVar>('reveal_env_var', { serverId, appId, key }),
  deleteEnvVar: (serverId: string, appId: string, key: string) =>
    invoke<void>('delete_env_var', { serverId, appId, key }),
  previewEnvDiff: (serverId: string, appId: string, dotenvContent: string) =>
    invoke<EnvDiffPreview>('preview_env_diff', { serverId, appId, dotenvContent }),
  applyEnvDiff: (serverId: string, appId: string, items: EnvDiffItem[]) =>
    invoke<EnvVar[]>('apply_env_diff', { serverId, appId, items }),

  // ---- Local Repo Discovery & Git Checkout Deploy ----
  discoverLocalRepos: (extraRoots?: string[]) =>
    invoke<DiscoveredRepo[]>('discover_local_repos', { extraRoots: extraRoots ?? null }),
  inspectLocalGit: (path: string) => invoke<LocalGitStatus>('inspect_local_git', { path }),
  deployLocalGit: (serverId: string, appId: string, path: string, dockerfilePath?: string) =>
    invoke<Deployment>('deploy_local_git', {
      serverId,
      appId,
      path,
      dockerfilePath: dockerfilePath ?? null,
    }),

  // ---- Container Terminal (WS in Rust -> Channel) ----
  openTerminal: (
    serverId: string,
    appId: string,
    sessionId: string,
    cols: number,
    rows: number,
    onFrame: (frame: TerminalFrame) => void,
  ) => {
    const channel = new Channel<TerminalFrame>()
    channel.onmessage = onFrame
    return invoke<void>('open_terminal', {
      serverId,
      appId,
      sessionId,
      cols,
      rows,
      onFrame: channel,
    })
  },
  writeTerminal: (sessionId: string, data: number[]) =>
    invoke<void>('write_terminal', { sessionId, data }),
  resizeTerminal: (sessionId: string, cols: number, rows: number) =>
    invoke<void>('resize_terminal', { sessionId, cols, rows }),
  closeTerminal: (sessionId: string) => invoke<void>('close_terminal', { sessionId }),

  // ---- Push Events from Rust ConnectionManager ----
  onServerStatus: (
    cb: (payload: { server_id: string; health: ServerHealth; error?: string | null }) => void,
  ): Promise<UnlistenFn> => listen('rig://server-status', (ev) => cb(ev.payload as never)),
  onServerEvent: (
    cb: (payload: { server_id: string; server_name: string; event: ServerEvent }) => void,
  ): Promise<UnlistenFn> => listen('rig://server-event', (ev) => cb(ev.payload as never)),
}
