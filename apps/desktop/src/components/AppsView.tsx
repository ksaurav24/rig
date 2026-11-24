import { useQuery, useQueryClient } from '@tanstack/react-query'
import {
  ArrowLeft,
  Box,
  FolderGit2,
  FolderPlus,
  Globe,
  KeyRound,
  Play,
  Plus,
  Rocket,
  RotateCcw,
  Save,
  ScrollText,
  Settings,
  Square,
  Terminal,
  Trash2,
  X,
} from 'lucide-react'
import { lazy, Suspense, useEffect, useState, type FormEvent } from 'react'
import { toast } from 'sonner'
import { EnvDiffPanel } from './EnvDiffPanel'
import { LocalGitPanel } from './LocalGitPanel'
import { LogStreamView } from './LogStreamView'
import { relativeTime } from '@/lib/format'
import { desktopApi } from '@/lib/ipc'
import type { App, Project } from '@/lib/types'

const TerminalTabs = lazy(() =>
  import('./TerminalTabs').then((m) => ({ default: m.TerminalTabs })),
)

export type AppDetailTab =
  | 'deployments'
  | 'logs'
  | 'env'
  | 'local-git'
  | 'domains'
  | 'terminal'
  | 'settings'

interface AppsViewProps {
  serverId: string
  projects: Project[]
  apps: App[]
  selectedAppId: string | null
  activeTab: AppDetailTab
  onSelectApp: (id: string | null) => void
  onSelectTab: (tab: AppDetailTab) => void
  onRefreshServer: () => void
  onDeployNew: () => void
}

export function AppsView({
  serverId,
  projects,
  apps,
  selectedAppId,
  activeTab,
  onSelectApp,
  onSelectTab,
  onRefreshServer,
  onDeployNew,
}: AppsViewProps) {
  const [projectFilter, setProjectFilter] = useState<string>('all')
  const [newProjectOpen, setNewProjectOpen] = useState(false)
  const [newProjectName, setNewProjectName] = useState('')
  const [newProjectDesc, setNewProjectDesc] = useState('')
  const [creatingProject, setCreatingProject] = useState(false)

  const [newDomainHost, setNewDomainHost] = useState('')
  const [newDomainTls, setNewDomainTls] = useState('auto')
  const [newDomainAccess, setNewDomainAccess] = useState('public')
  const [busyActionId, setBusyActionId] = useState<string | null>(null)
  const [inspectDeployId, setInspectDeployId] = useState<string | null>(null)
  const qc = useQueryClient()

  const detailQuery = useQuery({
    queryKey: ['desktop', serverId, 'app', selectedAppId],
    queryFn: () => desktopApi.getAppDetail(serverId, selectedAppId!),
    enabled: Boolean(selectedAppId),
  })

  // App Settings form state
  const detail = detailQuery.data
  const selectedApp = detail?.app ?? apps.find((a) => a.id === selectedAppId)
  const [editName, setEditName] = useState('')
  const [editImage, setEditImage] = useState('')
  const [editGitUrl, setEditGitUrl] = useState('')
  const [editGitBranch, setEditGitBranch] = useState('')
  const [editDockerfile, setEditDockerfile] = useState('')
  const [editPort, setEditPort] = useState(3000)
  const [editMemoryMb, setEditMemoryMb] = useState(512)
  const [editHealthPath, setEditHealthPath] = useState('')
  const [editAutoDeploy, setEditAutoDeploy] = useState(false)
  const [savingSettings, setSavingSettings] = useState(false)

  useEffect(() => {
    setInspectDeployId(null)
  }, [selectedAppId])

  useEffect(() => {
    if (!selectedApp) return
    setEditName(selectedApp.name)
    setEditImage(selectedApp.image ?? '')
    setEditGitUrl(selectedApp.git_url ?? '')
    setEditGitBranch(selectedApp.git_branch ?? '')
    setEditDockerfile(selectedApp.dockerfile_path ?? '')
    setEditPort(selectedApp.port)
    setEditMemoryMb(selectedApp.memory_mb ?? 512)
    setEditHealthPath(selectedApp.health_path ?? '')
    setEditAutoDeploy(selectedApp.auto_deploy)
  }, [selectedApp])

  async function runAction(appId: string, appName: string, action: 'deploy' | 'start' | 'restart' | 'stop') {
    setBusyActionId(`${appId}:${action}`)
    try {
      await desktopApi.appAction(serverId, appId, action)
      toast.success(`Triggered ${action} on ${appName}`)
      void qc.invalidateQueries({ queryKey: ['desktop', serverId, 'app', appId] })
      onRefreshServer()
    } catch (e) {
      toast.error(String(e))
    } finally {
      setBusyActionId(null)
    }
  }

  async function handleCreateProject(e: FormEvent) {
    e.preventDefault()
    if (!newProjectName.trim()) return
    setCreatingProject(true)
    try {
      const p = await desktopApi.createProject(serverId, newProjectName.trim(), newProjectDesc.trim())
      toast.success(`Created project "${p.name}"`)
      setNewProjectName('')
      setNewProjectDesc('')
      setNewProjectOpen(false)
      onRefreshServer()
    } catch (err) {
      toast.error(String(err))
    } finally {
      setCreatingProject(false)
    }
  }

  if (selectedAppId) {
    const app = selectedApp

    async function handleAddDomain(e: FormEvent) {
      e.preventDefault()
      if (!selectedAppId || !newDomainHost.trim()) return
      try {
        await desktopApi.addDomain(serverId, selectedAppId, {
          host: newDomainHost.trim(),
          tls_mode: newDomainTls,
          access: newDomainAccess,
        })
        setNewDomainHost('')
        toast.success('Domain added')
        void qc.invalidateQueries({ queryKey: ['desktop', serverId, 'app', selectedAppId] })
      } catch (err) {
        toast.error(String(err))
      }
    }

    async function handleSaveSettings(e: FormEvent) {
      e.preventDefault()
      if (!selectedAppId || !app) return
      setSavingSettings(true)
      try {
        await desktopApi.updateApp(serverId, selectedAppId, {
          name: editName.trim() || undefined,
          image: app.source_kind === 'image' ? editImage.trim() || undefined : undefined,
          git_url: app.source_kind === 'git' ? editGitUrl.trim() || undefined : undefined,
          git_branch: app.source_kind === 'git' ? editGitBranch.trim() || undefined : undefined,
          dockerfile_path: app.source_kind !== 'image' ? editDockerfile.trim() : undefined,
          port: Number(editPort) || app.port,
          memory_mb: Number(editMemoryMb) || undefined,
          health_path: editHealthPath.trim() || undefined,
          auto_deploy: editAutoDeploy,
        })
        toast.success('Saved app settings')
        void qc.invalidateQueries({ queryKey: ['desktop', serverId, 'app', selectedAppId] })
        onRefreshServer()
      } catch (err) {
        toast.error(String(err))
      } finally {
        setSavingSettings(false)
      }
    }

    async function handleDeleteApp() {
      if (!selectedAppId || !app) return
      setSavingSettings(true)
      try {
        await desktopApi.deleteApp(serverId, selectedAppId)
        toast.success(`Deleted ${app.name}`)
        onSelectApp(null)
        onRefreshServer()
      } catch (err) {
        toast.error(String(err))
      } finally {
        setSavingSettings(false)
      }
    }

    const tabs: { id: AppDetailTab; label: string; icon: React.ReactNode }[] = [
      { id: 'deployments', label: 'Deployments', icon: <Box className="size-3.5" /> },
      { id: 'logs', label: 'Logs', icon: <ScrollText className="size-3.5" /> },
      { id: 'env', label: `Env (${detail?.env.length ?? 0})`, icon: <KeyRound className="size-3.5" /> },
      { id: 'local-git', label: 'Local Git', icon: <FolderGit2 className="size-3.5" /> },
      { id: 'domains', label: `Domains (${detail?.domains.length ?? 0})`, icon: <Globe className="size-3.5" /> },
      { id: 'terminal', label: 'Terminal', icon: <Terminal className="size-3.5" /> },
      { id: 'settings', label: 'Settings', icon: <Settings className="size-3.5" /> },
    ]

    return (
      <div className="flex h-full min-h-0 flex-col gap-4">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex items-center gap-3">
            <button
              type="button"
              onClick={() => onSelectApp(null)}
              className="rounded-lg border border-line bg-surface p-1.5 text-muted hover:bg-surface-2 hover:text-fg"
            >
              <ArrowLeft className="size-4" />
            </button>
            <div>
              <div className="flex items-center gap-2">
                <h1 className="text-base font-semibold text-fg">{app?.name ?? 'Loading…'}</h1>
                {app && (
                  <span
                    className={`rounded px-1.5 py-0.5 font-mono text-[10px] uppercase ${
                      app.status === 'running'
                        ? 'bg-emerald-500/15 text-emerald-300'
                        : app.status === 'failed' || app.status === 'crashed'
                          ? 'bg-rose-500/15 text-rose-300'
                          : 'bg-surface-2 text-muted'
                    }`}
                  >
                    {app.status}
                  </span>
                )}
                {app?.project_name && (
                  <span className="rounded bg-surface-2 px-1.5 py-0.5 text-[10px] text-muted">
                    {app.project_name}
                  </span>
                )}
              </div>
              {app && (
                <div className="font-mono text-[11px] text-muted">
                  {app.source_kind} · port {app.port}
                  {app.host_port ? ` → :${app.host_port}` : ''}
                  {app.image ? ` · ${app.image}` : ''}
                  {app.git_url ? ` · ${app.git_url}` : ''}
                </div>
              )}
            </div>
          </div>

          <div className="flex items-center gap-1.5">
            {app?.source_kind === 'local' ? (
              <button
                type="button"
                onClick={() => onSelectTab('local-git')}
                className="inline-flex items-center gap-1.5 rounded-lg bg-emerald-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-emerald-500"
              >
                <Rocket className="size-3.5" />
                Deploy Local Folder
              </button>
            ) : (
              <button
                type="button"
                disabled={busyActionId !== null}
                onClick={() => app && void runAction(app.id, app.name, 'deploy')}
                className="inline-flex items-center gap-1.5 rounded-lg bg-emerald-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-emerald-500 disabled:opacity-50"
              >
                <Play className="size-3.5" />
                Deploy
              </button>
            )}
            <button
              type="button"
              disabled={busyActionId !== null}
              onClick={() => app && void runAction(app.id, app.name, 'restart')}
              className="inline-flex items-center gap-1.5 rounded-lg border border-line bg-surface px-3 py-1.5 text-xs text-fg hover:bg-surface-2 disabled:opacity-50"
            >
              <RotateCcw className="size-3.5" />
              Restart
            </button>
            {app?.status === 'stopped' ? (
              <button
                type="button"
                disabled={busyActionId !== null}
                onClick={() => app && void runAction(app.id, app.name, 'start')}
                className="inline-flex items-center gap-1.5 rounded-lg border border-line bg-surface px-3 py-1.5 text-xs text-emerald-300 hover:bg-emerald-500/10 disabled:opacity-50"
              >
                <Play className="size-3.5" />
                Start
              </button>
            ) : (
              <button
                type="button"
                disabled={busyActionId !== null}
                onClick={() => app && void runAction(app.id, app.name, 'stop')}
                className="inline-flex items-center gap-1.5 rounded-lg border border-line bg-surface px-3 py-1.5 text-xs text-rose-300 hover:bg-rose-500/10 disabled:opacity-50"
              >
                <Square className="size-3.5" />
                Stop
              </button>
            )}
          </div>
        </div>

        {/* App Sub-Navigation */}
        <div className="flex flex-wrap gap-1 border-b border-line pb-2">
          {tabs.map((t) => (
            <button
              key={t.id}
              type="button"
              onClick={() => onSelectTab(t.id)}
              className={`inline-flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-xs font-medium transition-colors ${
                activeTab === t.id
                  ? 'bg-surface-2 text-fg ring-1 ring-line'
                  : 'text-muted hover:bg-surface hover:text-fg'
              }`}
            >
              {t.icon}
              {t.label}
            </button>
          ))}
        </div>

        <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
          {activeTab === 'deployments' && (
            <div className="flex min-h-0 flex-1 flex-col gap-4">
              {inspectDeployId && (
                <div className="flex h-72 flex-col gap-1.5">
                  <div className="flex items-center justify-between text-xs">
                    <span className="font-mono text-muted">
                      Build & Deploy Logs · {inspectDeployId.slice(-8)}
                    </span>
                    <button
                      type="button"
                      onClick={() => setInspectDeployId(null)}
                      className="inline-flex items-center gap-1 rounded px-2 py-0.5 text-muted hover:bg-surface-2 hover:text-fg"
                    >
                      <X className="size-3" />
                      Close Logs
                    </button>
                  </div>
                  <LogStreamView
                    mode="deploy"
                    serverId={serverId}
                    deploymentId={inspectDeployId}
                    title={`Deploy ${inspectDeployId.slice(-8)}`}
                  />
                </div>
              )}

              <div className="overflow-hidden rounded-xl border border-line bg-surface">
                <div className="divide-y divide-line">
                  {(detail?.deployments ?? []).length === 0 ? (
                    <div className="px-4 py-8 text-center text-xs text-muted">
                      No deployments yet. Click Deploy or use the Local Git tab.
                    </div>
                  ) : (
                    (detail?.deployments ?? []).map((dep) => {
                      const inProgress =
                        dep.status === 'queued' ||
                        dep.status === 'building' ||
                        dep.status === 'deploying'
                      return (
                        <div
                          key={dep.id}
                          className="flex flex-wrap items-center justify-between gap-3 px-4 py-3 text-xs"
                        >
                          <div className="min-w-0">
                            <div className="flex items-center gap-2 font-mono">
                              <span className="font-semibold text-fg">{dep.id.slice(-8)}</span>
                              <span
                                className={`rounded px-1.5 py-0.5 text-[10px] uppercase ${
                                  dep.status === 'running'
                                    ? 'bg-emerald-500/15 text-emerald-300'
                                    : dep.status === 'failed'
                                      ? 'bg-rose-500/15 text-rose-300'
                                      : inProgress
                                        ? 'bg-amber-500/15 text-amber-300'
                                        : 'bg-surface-2 text-muted'
                                }`}
                              >
                                {dep.status}
                              </span>
                              <span className="text-[11px] text-muted">{dep.trigger}</span>
                              {dep.commit_sha && (
                                <span className="text-muted">{dep.commit_sha.slice(0, 8)}</span>
                              )}
                            </div>
                            {dep.commit_msg && (
                              <div className="mt-0.5 truncate text-[11px] text-muted">
                                {dep.commit_msg}
                              </div>
                            )}
                            {dep.error && (
                              <div className="mt-1 text-[11px] text-rose-300">{dep.error}</div>
                            )}
                          </div>
                          <div className="flex items-center gap-2">
                            <span className="font-mono text-[11px] text-muted">
                              {relativeTime(dep.created_at)}
                            </span>
                            <button
                              type="button"
                              onClick={() =>
                                setInspectDeployId((cur) => (cur === dep.id ? null : dep.id))
                              }
                              className="inline-flex items-center gap-1 rounded-md border border-line bg-surface-2 px-2 py-1 text-[11px] text-fg hover:bg-bg"
                            >
                              <ScrollText className="size-3" />
                              Build Logs
                            </button>
                            {inProgress && (
                              <button
                                type="button"
                                onClick={async () => {
                                  try {
                                    await desktopApi.cancelDeployment(serverId, dep.id)
                                    toast.success('Deployment cancelled')
                                    void detailQuery.refetch()
                                  } catch (e) {
                                    toast.error(String(e))
                                  }
                                }}
                                className="rounded-md border border-rose-500/30 bg-rose-500/10 px-2 py-1 text-[11px] text-rose-300 hover:bg-rose-500/20"
                              >
                                Cancel
                              </button>
                            )}
                            {dep.status === 'superseded' && (
                              <button
                                type="button"
                                onClick={async () => {
                                  try {
                                    await desktopApi.rollbackDeployment(serverId, dep.id)
                                    toast.success('Rollback started')
                                    void detailQuery.refetch()
                                  } catch (e) {
                                    toast.error(String(e))
                                  }
                                }}
                                className="rounded-md border border-line bg-surface-2 px-2 py-1 text-[11px] text-fg hover:bg-bg"
                              >
                                Rollback
                              </button>
                            )}
                          </div>
                        </div>
                      )
                    })
                  )}
                </div>
              </div>
            </div>
          )}

          {activeTab === 'logs' && (
            <LogStreamView
              mode="app"
              serverId={serverId}
              appId={selectedAppId}
              title={`${app?.name ?? 'App'} · Container Logs`}
            />
          )}

          {activeTab === 'env' && (
            <EnvDiffPanel
              serverId={serverId}
              appId={selectedAppId}
              env={detail?.env ?? []}
              onApplied={() => void detailQuery.refetch()}
            />
          )}

          {activeTab === 'local-git' && (
            <LocalGitPanel
              serverId={serverId}
              appId={selectedAppId}
              appName={app?.name ?? 'App'}
              defaultDockerfile={app?.dockerfile_path ?? 'Dockerfile'}
              onDeployed={() => {
                onSelectTab('deployments')
                void detailQuery.refetch()
              }}
            />
          )}

          {activeTab === 'domains' && (
            <div className="space-y-4">
              <form
                onSubmit={handleAddDomain}
                className="flex flex-wrap items-center gap-2 rounded-xl border border-line bg-surface p-3.5 text-xs"
              >
                <input
                  type="text"
                  required
                  value={newDomainHost}
                  onChange={(e) => setNewDomainHost(e.target.value)}
                  placeholder="app.example.com"
                  className="min-w-[200px] flex-1 rounded-lg border border-line bg-bg px-3 py-1.5 font-mono text-xs text-fg outline-none focus:border-emerald-500"
                />
                <select
                  value={newDomainTls}
                  onChange={(e) => setNewDomainTls(e.target.value)}
                  className="rounded-lg border border-line bg-bg px-2.5 py-1.5 text-xs text-fg"
                >
                  <option value="auto">TLS: Auto</option>
                  <option value="internal">TLS: Internal CA</option>
                  <option value="tailscale">TLS: Tailscale</option>
                  <option value="off">TLS: Off</option>
                </select>
                <select
                  value={newDomainAccess}
                  onChange={(e) => setNewDomainAccess(e.target.value)}
                  className="rounded-lg border border-line bg-bg px-2.5 py-1.5 text-xs text-fg"
                >
                  <option value="public">Public</option>
                  <option value="lan">LAN</option>
                  <option value="tailscale">Tailscale</option>
                </select>
                <button
                  type="submit"
                  className="inline-flex items-center gap-1 rounded-lg bg-emerald-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-emerald-500"
                >
                  <Plus className="size-3.5" />
                  Add Domain
                </button>
              </form>

              <div className="divide-y divide-line overflow-hidden rounded-xl border border-line bg-surface">
                {(detail?.domains ?? []).length === 0 ? (
                  <div className="px-4 py-8 text-center text-xs text-muted">
                    No domains attached yet.
                  </div>
                ) : (
                  (detail?.domains ?? []).map((dom) => (
                    <div
                      key={dom.id}
                      className="flex items-center justify-between px-4 py-2.5 text-xs"
                    >
                      <div>
                        <div className="font-mono font-medium text-fg">{dom.host}</div>
                        <div className="text-[11px] text-muted">
                          TLS: {dom.tls_mode} · Access: {dom.access}
                          {dom.port ? ` · Port ${dom.port}` : ''}
                        </div>
                      </div>
                      <button
                        type="button"
                        onClick={async () => {
                          try {
                            await desktopApi.deleteDomain(serverId, selectedAppId, dom.id)
                            toast.success(`Removed ${dom.host}`)
                            void detailQuery.refetch()
                          } catch (e) {
                            toast.error(String(e))
                          }
                        }}
                        className="rounded-md p-1.5 text-muted hover:bg-rose-500/15 hover:text-rose-300"
                      >
                        <Trash2 className="size-3.5" />
                      </button>
                    </div>
                  ))
                )}
              </div>
            </div>
          )}

          {activeTab === 'terminal' && (
            <Suspense
              fallback={
                <div className="flex flex-1 items-center justify-center rounded-xl border border-line bg-surface p-8 text-xs text-muted">
                  Loading terminal…
                </div>
              }
            >
              <TerminalTabs
                serverId={serverId}
                appId={selectedAppId}
                appName={app?.name ?? 'App'}
                isRunning={app?.status === 'running'}
              />
            </Suspense>
          )}

          {activeTab === 'settings' && app && (
            <div className="max-w-2xl space-y-5">
              <form
                onSubmit={handleSaveSettings}
                className="space-y-4 rounded-xl border border-line bg-surface p-4 text-xs"
              >
                <div className="text-sm font-semibold text-fg">Application Configuration</div>
                <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
                  <div>
                    <label className="mb-1 block text-muted">App Name</label>
                    <input
                      type="text"
                      required
                      value={editName}
                      onChange={(e) => setEditName(e.target.value)}
                      className="w-full rounded-lg border border-line bg-bg px-3 py-1.5 text-xs text-fg outline-none focus:border-emerald-500"
                    />
                  </div>
                  <div>
                    <label className="mb-1 block text-muted">Container Port</label>
                    <input
                      type="number"
                      min={1}
                      max={65535}
                      value={editPort}
                      onChange={(e) => setEditPort(Number(e.target.value))}
                      className="w-full rounded-lg border border-line bg-bg px-3 py-1.5 font-mono text-xs text-fg outline-none focus:border-emerald-500"
                    />
                  </div>
                  <div>
                    <label className="mb-1 block text-muted">Memory Limit (MB)</label>
                    <input
                      type="number"
                      min={64}
                      max={65536}
                      value={editMemoryMb}
                      onChange={(e) => setEditMemoryMb(Number(e.target.value))}
                      className="w-full rounded-lg border border-line bg-bg px-3 py-1.5 font-mono text-xs text-fg outline-none focus:border-emerald-500"
                    />
                  </div>
                  <div>
                    <label className="mb-1 block text-muted">Health Check Path</label>
                    <input
                      type="text"
                      value={editHealthPath}
                      onChange={(e) => setEditHealthPath(e.target.value)}
                      placeholder="/healthz"
                      className="w-full rounded-lg border border-line bg-bg px-3 py-1.5 font-mono text-xs text-fg outline-none focus:border-emerald-500"
                    />
                  </div>
                </div>

                {app.source_kind === 'image' && (
                  <div>
                    <label className="mb-1 block text-muted">Container Image</label>
                    <input
                      type="text"
                      value={editImage}
                      onChange={(e) => setEditImage(e.target.value)}
                      className="w-full rounded-lg border border-line bg-bg px-3 py-1.5 font-mono text-xs text-fg outline-none focus:border-emerald-500"
                    />
                  </div>
                )}

                {app.source_kind === 'git' && (
                  <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
                    <div className="sm:col-span-2">
                      <label className="mb-1 block text-muted">Git Repository URL</label>
                      <input
                        type="text"
                        value={editGitUrl}
                        onChange={(e) => setEditGitUrl(e.target.value)}
                        className="w-full rounded-lg border border-line bg-bg px-3 py-1.5 font-mono text-xs text-fg outline-none focus:border-emerald-500"
                      />
                    </div>
                    <div>
                      <label className="mb-1 block text-muted">Branch</label>
                      <input
                        type="text"
                        value={editGitBranch}
                        onChange={(e) => setEditGitBranch(e.target.value)}
                        className="w-full rounded-lg border border-line bg-bg px-3 py-1.5 font-mono text-xs text-fg outline-none focus:border-emerald-500"
                      />
                    </div>
                  </div>
                )}

                {app.source_kind !== 'image' && (
                  <div>
                    <label className="mb-1 block text-muted">
                      Dockerfile Path <span className="opacity-70">(empty = Railpack)</span>
                    </label>
                    <input
                      type="text"
                      value={editDockerfile}
                      onChange={(e) => setEditDockerfile(e.target.value)}
                      placeholder="Dockerfile"
                      className="w-full rounded-lg border border-line bg-bg px-3 py-1.5 font-mono text-xs text-fg outline-none focus:border-emerald-500"
                    />
                  </div>
                )}

                {app.source_kind === 'git' && (
                  <label className="inline-flex cursor-pointer items-center gap-2 text-fg">
                    <input
                      type="checkbox"
                      checked={editAutoDeploy}
                      onChange={(e) => setEditAutoDeploy(e.target.checked)}
                      className="accent-emerald-500"
                    />
                    Auto-deploy on Git push webhook
                  </label>
                )}

                <div className="flex justify-end border-t border-line pt-3">
                  <button
                    type="submit"
                    disabled={savingSettings}
                    className="inline-flex items-center gap-1.5 rounded-lg bg-emerald-600 px-4 py-1.5 text-xs font-medium text-white hover:bg-emerald-500 disabled:opacity-50"
                  >
                    <Save className="size-3.5" />
                    {savingSettings ? 'Saving…' : 'Save Changes'}
                  </button>
                </div>
              </form>

              <div className="flex items-center justify-between rounded-xl border border-rose-500/25 bg-rose-500/5 px-4 py-3 text-xs">
                <div>
                  <div className="font-semibold text-rose-300">Delete Application</div>
                  <div className="text-muted">
                    Stops containers and permanently removes {app.name}, its deployments, and domains.
                  </div>
                </div>
                <button
                  type="button"
                  disabled={savingSettings}
                  onClick={() => void handleDeleteApp()}
                  className="inline-flex items-center gap-1.5 rounded-lg bg-rose-600 px-3 py-1.5 font-medium text-white hover:bg-rose-500 disabled:opacity-50"
                >
                  <Trash2 className="size-3.5" />
                  Delete App
                </button>
              </div>
            </div>
          )}
        </div>
      </div>
    )
  }

  const filteredApps =
    projectFilter === 'all' ? apps : apps.filter((a) => a.project_id === projectFilter)

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h1 className="text-lg font-semibold tracking-tight text-fg">Applications</h1>
        <div className="flex flex-wrap items-center gap-2">
          <button
            type="button"
            onClick={() => setNewProjectOpen((v) => !v)}
            className="inline-flex items-center gap-1.5 rounded-lg border border-line bg-surface px-3 py-1.5 text-xs text-fg hover:bg-surface-2"
          >
            <FolderPlus className="size-3.5" />
            New Project
          </button>
          <button
            type="button"
            onClick={onDeployNew}
            className="inline-flex items-center gap-1.5 rounded-lg bg-emerald-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-emerald-500"
          >
            <Rocket className="size-3.5" />
            Deploy App
          </button>
        </div>
      </div>

      {newProjectOpen && (
        <form
          onSubmit={handleCreateProject}
          className="flex flex-wrap items-center gap-2 rounded-xl border border-emerald-500/30 bg-surface p-3.5 text-xs"
        >
          <input
            type="text"
            required
            value={newProjectName}
            onChange={(e) => setNewProjectName(e.target.value)}
            placeholder="Project name (e.g. Production, Side Projects)"
            className="w-60 rounded-lg border border-line bg-bg px-3 py-1.5 text-xs text-fg outline-none focus:border-emerald-500"
            autoFocus
          />
          <input
            type="text"
            value={newProjectDesc}
            onChange={(e) => setNewProjectDesc(e.target.value)}
            placeholder="Description (optional)"
            className="min-w-[200px] flex-1 rounded-lg border border-line bg-bg px-3 py-1.5 text-xs text-fg outline-none focus:border-emerald-500"
          />
          <button
            type="submit"
            disabled={creatingProject || !newProjectName.trim()}
            className="inline-flex items-center gap-1 rounded-lg bg-emerald-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-emerald-500 disabled:opacity-50"
          >
            <Plus className="size-3.5" />
            {creatingProject ? 'Creating…' : 'Create Project'}
          </button>
          <button
            type="button"
            onClick={() => setNewProjectOpen(false)}
            className="rounded-lg p-1.5 text-muted hover:bg-surface-2 hover:text-fg"
          >
            <X className="size-3.5" />
          </button>
        </form>
      )}

      {/* Project Filter Pills */}
      <div className="flex flex-wrap items-center gap-1.5">
        <button
          type="button"
          onClick={() => setProjectFilter('all')}
          className={`rounded-lg px-2.5 py-1 text-xs ${
            projectFilter === 'all'
              ? 'bg-surface-2 text-fg ring-1 ring-line'
              : 'text-muted hover:text-fg'
          }`}
        >
          All ({apps.length})
        </button>
        {projects.map((p) => (
          <button
            key={p.id}
            type="button"
            onClick={() => setProjectFilter(p.id)}
            className={`rounded-lg px-2.5 py-1 text-xs ${
              projectFilter === p.id
                ? 'bg-surface-2 text-fg ring-1 ring-line'
                : 'text-muted hover:text-fg'
            }`}
          >
            {p.name} ({p.app_count})
          </button>
        ))}
      </div>

      <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
        {filteredApps.length === 0 ? (
          <div className="col-span-full flex flex-col items-center justify-center rounded-xl border border-line bg-surface p-10 text-center">
            <div className="mb-3 flex size-10 items-center justify-center rounded-xl bg-emerald-500/15 text-emerald-400">
              <Rocket className="size-5" />
            </div>
            <div className="text-sm font-medium text-fg">No applications yet</div>
            <p className="mt-1 max-w-sm text-xs text-muted">
              Deploy a local repository from this laptop, a Docker image, or a Git repository.
            </p>
            <button
              type="button"
              onClick={onDeployNew}
              className="mt-4 inline-flex items-center gap-1.5 rounded-lg bg-emerald-600 px-4 py-2 text-xs font-medium text-white hover:bg-emerald-500"
            >
              <Plus className="size-3.5" />
              Deploy Your First App
            </button>
          </div>
        ) : (
          filteredApps.map((app) => (
            <div
              key={app.id}
              className="flex flex-col justify-between rounded-xl border border-line bg-surface p-4 transition-colors hover:border-emerald-500/40"
            >
              <button
                type="button"
                onClick={() => onSelectApp(app.id)}
                className="text-left"
              >
                <div className="flex items-center justify-between gap-2">
                  <div className="flex items-center gap-2">
                    <span
                      className={`size-2 rounded-full ${
                        app.status === 'running'
                          ? 'bg-green-400'
                          : app.status === 'failed' || app.status === 'crashed'
                            ? 'bg-rose-500'
                            : 'bg-amber-400'
                      }`}
                    />
                    <span className="text-sm font-semibold text-fg">{app.name}</span>
                  </div>
                  <span className="rounded bg-bg px-1.5 py-0.5 font-mono text-[10px] text-muted uppercase">
                    {app.status}
                  </span>
                </div>
                <div className="mt-1.5 font-mono text-[11px] text-muted">
                  {app.project_name} · {app.source_kind} · port {app.port}
                  {app.git_branch ? ` · ${app.git_branch}` : ''}
                </div>
                {app.domains.length > 0 && (
                  <div className="mt-1.5 flex flex-wrap gap-1">
                    {app.domains.slice(0, 2).map((d) => (
                      <span
                        key={d}
                        className="rounded bg-surface-2 px-1.5 py-0.5 font-mono text-[10px] text-emerald-300"
                      >
                        {d}
                      </span>
                    ))}
                  </div>
                )}
                {app.last_crash?.reason && (
                  <div className="mt-2 truncate rounded bg-rose-500/10 px-2 py-1 text-[11px] text-rose-300">
                    {app.last_crash.reason}
                  </div>
                )}
              </button>

              {/* Quick Actions Bar */}
              <div className="mt-3 flex flex-wrap items-center gap-1.5 border-t border-line pt-2.5 text-[11px]">
                <button
                  type="button"
                  disabled={busyActionId !== null}
                  onClick={() => {
                    if (app.source_kind === 'local') {
                      onSelectApp(app.id)
                      onSelectTab('local-git')
                    } else {
                      void runAction(app.id, app.name, 'deploy')
                    }
                  }}
                  className="inline-flex items-center gap-1 rounded-md bg-emerald-600/90 px-2 py-1 font-medium text-white hover:bg-emerald-500 disabled:opacity-50"
                >
                  <Play className="size-3" />
                  Deploy
                </button>
                <button
                  type="button"
                  disabled={busyActionId !== null}
                  onClick={() => void runAction(app.id, app.name, 'restart')}
                  className="inline-flex items-center gap-1 rounded-md border border-line bg-bg px-2 py-1 text-fg hover:bg-surface-2 disabled:opacity-50"
                >
                  <RotateCcw className="size-3" />
                  Restart
                </button>
                <button
                  type="button"
                  onClick={() => {
                    onSelectApp(app.id)
                    onSelectTab('logs')
                  }}
                  className="inline-flex items-center gap-1 rounded-md border border-line bg-bg px-2 py-1 text-fg hover:bg-surface-2"
                >
                  <ScrollText className="size-3" />
                  Logs
                </button>
                <button
                  type="button"
                  onClick={() => {
                    onSelectApp(app.id)
                    onSelectTab('env')
                  }}
                  className="inline-flex items-center gap-1 rounded-md border border-line bg-bg px-2 py-1 text-fg hover:bg-surface-2"
                >
                  <KeyRound className="size-3" />
                  Env
                </button>
                {app.status === 'running' && (
                  <button
                    type="button"
                    onClick={() => {
                      onSelectApp(app.id)
                      onSelectTab('terminal')
                    }}
                    className="ml-auto inline-flex items-center gap-1 rounded-md border border-line bg-bg px-2 py-1 text-sky-300 hover:bg-surface-2"
                  >
                    <Terminal className="size-3" />
                    Shell
                  </button>
                )}
              </div>
            </div>
          ))
        )}
      </div>
    </div>
  )
}
