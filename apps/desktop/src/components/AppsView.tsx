import { useQuery, useQueryClient } from '@tanstack/react-query'
import {
  ArrowLeft,
  Box,
  Copy,
  ExternalLink,
  FolderGit2,
  FolderPlus,
  GitBranch,
  Globe,
  KeyRound,
  Play,
  Plus,
  Rocket,
  RotateCcw,
  Save,
  ScrollText,
  Search,
  Settings,
  ShieldCheck,
  Square,
  Terminal,
  Trash2,
  X,
} from 'lucide-react'
import { lazy, Suspense, useEffect, useMemo, useState, type ComponentType, type FormEvent } from 'react'
import { toast } from 'sonner'
import { EnvDiffPanel } from './EnvDiffPanel'
import { LocalGitPanel } from './LocalGitPanel'
import { LogStreamView } from './LogStreamView'
import { Page, Panel } from '@rig/ui/app/page'
import { EmptyState, Status, statusTone } from '@rig/ui/app/state'
import { Badge } from '@rig/ui/badge'
import { Banner, BannerDescription } from '@rig/ui/banner'
import { Button } from '@rig/ui/button'
import { Card, CardContent, CardFooter, CardGroup, CardHeader, CardTitle } from '@rig/ui/card'
import {
  Combobox,
  ComboboxContent,
  ComboboxEmpty,
  ComboboxInput,
  ComboboxItem,
  ComboboxList,
} from '@rig/ui/combobox'
import { ConfirmDialog } from '@rig/ui/confirm-dialog'
import { Field, FieldLabel } from '@rig/ui/field'
import { Input } from '@rig/ui/input'
import { Select, SelectContent, SelectItem, SelectTrigger } from '@rig/ui/select'
import { Switch } from '@rig/ui/switch'
import { Tabs, TabsItem } from '@rig/ui/tabs'
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
  const [appSearch, setAppSearch] = useState('')
  const [newProjectOpen, setNewProjectOpen] = useState(false)
  const [newProjectName, setNewProjectName] = useState('')
  const [newProjectDesc, setNewProjectDesc] = useState('')
  const [creatingProject, setCreatingProject] = useState(false)

  const [confirmState, setConfirmState] = useState<{
    title: string
    description: string
    target?: string
    confirmLabel: string
    tone: 'danger' | 'warning'
    action: () => Promise<void>
  } | null>(null)

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

  const parsedGithubRepo = useMemo(() => {
    const raw = (editGitUrl || selectedApp?.git_url || '').trim()
    const m = /^https:\/\/github\.com\/([A-Za-z0-9_.-]+)\/([A-Za-z0-9_.-]+?)(?:\.git)?\/?$/.exec(raw)
    return m ? { owner: m[1], repo: m[2] } : null
  }, [editGitUrl, selectedApp?.git_url])

  const appBranchesQuery = useQuery({
    queryKey: ['desktop', serverId, 'app-gh-branches', parsedGithubRepo?.owner, parsedGithubRepo?.repo],
    queryFn: () =>
      desktopApi.listGithubBranches(serverId, parsedGithubRepo!.owner, parsedGithubRepo!.repo),
    enabled: Boolean(
      selectedAppId &&
        activeTab === 'settings' &&
        selectedApp?.source_kind === 'git' &&
        parsedGithubRepo,
    ),
    retry: false,
  })

  const appBranchItems = useMemo(
    () =>
      (appBranchesQuery.data ?? []).map((b) => ({
        value: b.name,
        label: b.name,
        branch: b,
      })),
    [appBranchesQuery.data],
  )

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

  async function runAction(
    appId: string,
    appName: string,
    action: 'deploy' | 'start' | 'restart' | 'stop',
  ) {
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

    const tabs: {
      id: AppDetailTab
      label: string
      icon: ComponentType<{ size?: number; strokeWidth?: number; className?: string }>
    }[] = [
      { id: 'deployments', label: 'Deployments', icon: Box },
      { id: 'logs', label: 'Logs', icon: ScrollText },
      { id: 'env', label: `Env (${detail?.env.length ?? 0})`, icon: KeyRound },
      ...(app?.source_kind !== 'image'
        ? [{ id: 'local-git' as const, label: 'Local Git', icon: FolderGit2 }]
        : []),
      { id: 'domains', label: `Domains (${detail?.domains.length ?? 0})`, icon: Globe },
      { id: 'terminal', label: 'Terminal', icon: Terminal },
      { id: 'settings', label: 'Settings', icon: Settings },
    ]
    const activeTabIndex = Math.max(
      0,
      tabs.findIndex((t) => t.id === activeTab),
    )

    return (
      <div className="mx-auto flex h-full min-h-0 w-full max-w-6xl flex-col gap-5 px-4 py-6 md:px-8 md:py-8">
        <div className="flex flex-wrap items-center justify-between gap-4">
          <div className="flex min-w-0 items-center gap-3">
            <Button
              type="button"
              variant="tertiary"
              size="icon"
              onClick={() => onSelectApp(null)}
              aria-label="Back to applications"
            >
              <ArrowLeft />
            </Button>
            <div className="min-w-0">
              <div className="flex flex-wrap items-center gap-2.5">
                <h1 className="truncate text-[20px] font-semibold tracking-tight text-foreground">
                  {app?.name ?? 'Loading…'}
                </h1>
                {app && (
                  <Status tone={statusTone(app.status)} variant="dot">
                    {app.status}
                  </Status>
                )}
                {app?.project_name && (
                  <Badge variant="solid" color="gray" size="compact">
                    {app.project_name}
                  </Badge>
                )}
              </div>
              {app && (
                <div className="mt-0.5 truncate font-mono text-[12.5px] text-muted-foreground">
                  {app.source_kind} · port {app.port}
                  {app.host_port ? ` → :${app.host_port}` : ''}
                  {app.image ? ` · ${app.image}` : ''}
                  {app.git_url ? ` · ${app.git_url}` : ''}
                </div>
              )}
            </div>
          </div>

          <div className="flex flex-wrap items-center gap-2">
            {app?.source_kind === 'local' ? (
              <Button
                type="button"
                variant="primary"
                leadingIcon={Rocket}
                onClick={() => onSelectTab('local-git')}
              >
                Deploy Local Folder
              </Button>
            ) : (
              <Button
                type="button"
                variant="primary"
                leadingIcon={Play}
                loading={busyActionId === `${app?.id}:deploy`}
                disabled={busyActionId !== null}
                onClick={() => app && void runAction(app.id, app.name, 'deploy')}
              >
                Deploy
              </Button>
            )}
            <Button
              type="button"
              variant="tertiary"
              leadingIcon={RotateCcw}
              loading={busyActionId === `${app?.id}:restart`}
              disabled={busyActionId !== null}
              onClick={() => app && void runAction(app.id, app.name, 'restart')}
            >
              Restart
            </Button>
            {app?.status === 'stopped' ? (
              <Button
                type="button"
                variant="secondary"
                leadingIcon={Play}
                disabled={busyActionId !== null}
                onClick={() => app && void runAction(app.id, app.name, 'start')}
              >
                Start
              </Button>
            ) : (
              <Button
                type="button"
                variant="destructive"
                leadingIcon={Square}
                disabled={busyActionId !== null}
                onClick={() =>
                  app &&
                  setConfirmState({
                    title: 'Stop Application',
                    description:
                      'Stops the running container immediately. Traffic routed to this application will fail until you start or deploy it again.',
                    target: app.name,
                    confirmLabel: 'Stop App',
                    tone: 'warning',
                    action: () => runAction(app.id, app.name, 'stop'),
                  })
                }
              >
                Stop
              </Button>
            )}
          </div>
        </div>

        {/* Fluid App Sub-Navigation Tabs */}
        <div className="border-b border-border/80 pb-2.5">
          <Tabs
            selectedIndex={activeTabIndex}
            onSelect={(idx) => {
              const target = tabs[idx]
              if (target) onSelectTab(target.id)
            }}
          >
            {tabs.map((t, idx) => (
              <TabsItem key={t.id} index={idx} label={t.label} icon={t.icon} />
            ))}
          </Tabs>
        </div>

        <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
          {activeTab === 'deployments' && (
            <div className="flex min-h-0 flex-1 flex-col gap-5">
              {inspectDeployId && (
                <div className="flex h-80 flex-col gap-2">
                  <div className="flex items-center justify-between text-[13px]">
                    <span className="font-mono text-muted-foreground">
                      Build & Deploy Logs · {inspectDeployId.slice(-8)}
                    </span>
                    <Button
                      type="button"
                      variant="ghost"
                      size="compact"
                      leadingIcon={X}
                      onClick={() => setInspectDeployId(null)}
                    >
                      Close Logs
                    </Button>
                  </div>
                  <LogStreamView
                    mode="deploy"
                    serverId={serverId}
                    deploymentId={inspectDeployId}
                    title={`Deploy ${inspectDeployId.slice(-8)}`}
                  />
                </div>
              )}

              <Panel flush>
                <div className="divide-y divide-border/80">
                  {(detail?.deployments ?? []).length === 0 ? (
                    <div className="p-6">
                      <EmptyState icon={Box} title="No deployments yet">
                        Click Deploy above or upload directly from the Local Git tab.
                      </EmptyState>
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
                          className="flex flex-wrap items-center justify-between gap-3 px-5 py-3.5 text-[13.5px]"
                        >
                          <div className="min-w-0">
                            <div className="flex flex-wrap items-center gap-2.5">
                              <span className="font-mono font-semibold text-foreground">
                                {dep.id.slice(-8)}
                              </span>
                              <Status tone={statusTone(dep.status)} size="compact">
                                {dep.status}
                              </Status>
                              <Badge variant="solid" color="gray" size="compact">
                                {dep.trigger}
                              </Badge>
                              {dep.commit_sha && (
                                <span className="font-mono text-[12px] text-muted-foreground">
                                  {dep.commit_sha.slice(0, 8)}
                                </span>
                              )}
                            </div>
                            {dep.commit_msg && (
                              <div className="mt-1 truncate text-[13px] text-muted-foreground">
                                {dep.commit_msg}
                              </div>
                            )}
                            {dep.error && (
                              <div className="mt-1.5 text-[12.5px] text-fail">{dep.error}</div>
                            )}
                          </div>
                          <div className="flex flex-wrap items-center gap-2">
                            <span className="font-mono text-[12px] text-muted-foreground tabular-nums">
                              {relativeTime(dep.created_at)}
                            </span>
                            <Button
                              type="button"
                              variant="tertiary"
                              size="compact"
                              leadingIcon={ScrollText}
                              active={inspectDeployId === dep.id}
                              onClick={() =>
                                setInspectDeployId((cur) => (cur === dep.id ? null : dep.id))
                              }
                            >
                              Build Logs
                            </Button>
                            {inProgress && (
                              <Button
                                type="button"
                                variant="destructive"
                                size="compact"
                                onClick={() =>
                                  setConfirmState({
                                    title: 'Cancel Deployment',
                                    description:
                                      'Aborts this in-progress build and deployment. The previous release will remain active.',
                                    target: `Deployment ${dep.id.slice(-8)}`,
                                    confirmLabel: 'Cancel Deployment',
                                    tone: 'danger',
                                    action: async () => {
                                      try {
                                        await desktopApi.cancelDeployment(serverId, dep.id)
                                        toast.success('Deployment cancelled')
                                        void detailQuery.refetch()
                                      } catch (e) {
                                        toast.error(String(e))
                                      }
                                    },
                                  })
                                }
                              >
                                Cancel
                              </Button>
                            )}
                            {dep.status === 'superseded' && (
                              <Button
                                type="button"
                                variant="secondary"
                                size="compact"
                                onClick={() =>
                                  setConfirmState({
                                    title: 'Rollback Deployment',
                                    description:
                                      'Replaces the current live container with this earlier deployment artifact.',
                                    target: `Deployment ${dep.id.slice(-8)}`,
                                    confirmLabel: 'Rollback',
                                    tone: 'warning',
                                    action: async () => {
                                      try {
                                        await desktopApi.rollbackDeployment(serverId, dep.id)
                                        toast.success('Rollback started')
                                        void detailQuery.refetch()
                                      } catch (e) {
                                        toast.error(String(e))
                                      }
                                    },
                                  })
                                }
                              >
                                Rollback
                              </Button>
                            )}
                          </div>
                        </div>
                      )
                    })
                  )}
                </div>
              </Panel>
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
            <div className="space-y-5">
              <Panel>
                <form onSubmit={handleAddDomain} className="flex flex-wrap items-center gap-2.5">
                  <Input
                    type="text"
                    required
                    value={newDomainHost}
                    onChange={(e) => setNewDomainHost(e.target.value)}
                    placeholder="app.example.com"
                    className="min-w-[220px] flex-1 font-mono"
                  />
                  <Select value={newDomainTls} onValueChange={setNewDomainTls}>
                    <SelectTrigger className="w-[175px]" placeholder="TLS Mode" />
                    <SelectContent>
                      <SelectItem index={0} value="auto">TLS: Auto</SelectItem>
                      <SelectItem index={1} value="internal">TLS: Internal CA</SelectItem>
                      <SelectItem index={2} value="tailscale">TLS: Tailscale</SelectItem>
                      <SelectItem index={3} value="off">TLS: Off</SelectItem>
                    </SelectContent>
                  </Select>
                  <Select value={newDomainAccess} onValueChange={setNewDomainAccess}>
                    <SelectTrigger className="w-[140px]" placeholder="Access" />
                    <SelectContent>
                      <SelectItem index={0} value="public">Public</SelectItem>
                      <SelectItem index={1} value="lan">LAN</SelectItem>
                      <SelectItem index={2} value="tailscale">Tailscale</SelectItem>
                    </SelectContent>
                  </Select>
                  <Button type="submit" variant="primary" leadingIcon={Plus}>
                    Add Domain
                  </Button>
                </form>
              </Panel>

              <Panel flush>
                {(detail?.domains ?? []).length === 0 ? (
                  <div className="p-6">
                    <EmptyState icon={Globe} title="No domains attached yet">
                      Attach a custom domain or internal hostname routed via Caddy.
                    </EmptyState>
                  </div>
                ) : (
                  <div className="divide-y divide-border/80">
                    {(detail?.domains ?? []).map((dom) => {
                      const url = `${dom.tls_mode === 'off' ? 'http' : 'https'}://${dom.host}${dom.port ? `:${dom.port}` : ''}`
                      return (
                        <div
                          key={dom.id}
                          className="flex items-center justify-between gap-3 px-5 py-3.5"
                        >
                          <div className="min-w-0">
                            <div className="truncate font-mono text-[14px] font-medium text-foreground">
                              {dom.host}
                            </div>
                            <div className="mt-0.5 text-[12.5px] text-muted-foreground">
                              TLS: {dom.tls_mode} · Access: {dom.access}
                              {dom.port ? ` · Port ${dom.port}` : ''}
                            </div>
                          </div>
                          <div className="flex shrink-0 items-center gap-1">
                            <Button
                              type="button"
                              variant="ghost"
                              size="icon-compact"
                              aria-label={`Open ${dom.host}`}
                              title={`Open ${url}`}
                              onClick={() => void desktopApi.openExternalUrl(url)}
                            >
                              <ExternalLink />
                            </Button>
                            <Button
                              type="button"
                              variant="ghost"
                              size="icon-compact"
                              aria-label={`Copy ${dom.host}`}
                              title="Copy domain URL"
                              onClick={() => {
                                void navigator.clipboard.writeText(url)
                                toast.success(`Copied ${url}`)
                              }}
                            >
                              <Copy />
                            </Button>
                            <Button
                              type="button"
                              variant="ghost"
                              size="icon-compact"
                              aria-label={`Remove ${dom.host}`}
                              onClick={() =>
                                setConfirmState({
                                  title: 'Remove Domain',
                                  description:
                                    'Detaches this hostname from Caddy routing and removes its TLS configuration.',
                                  target: dom.host,
                                  confirmLabel: 'Remove Domain',
                                  tone: 'danger',
                                  action: async () => {
                                    try {
                                      await desktopApi.deleteDomain(serverId, selectedAppId, dom.id)
                                      toast.success(`Removed ${dom.host}`)
                                      void detailQuery.refetch()
                                    } catch (e) {
                                      toast.error(String(e))
                                    }
                                  },
                                })
                              }
                              className="text-muted-foreground hover:text-fail"
                            >
                              <Trash2 />
                            </Button>
                          </div>
                        </div>
                      )
                    })}
                  </div>
                )}
              </Panel>
            </div>
          )}

          {activeTab === 'terminal' && (
            <Suspense
              fallback={
                <Panel className="flex flex-1 items-center justify-center py-12 text-[14px] text-muted-foreground">
                  Loading container terminal…
                </Panel>
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
              <Panel title="Application Configuration">
                <form onSubmit={handleSaveSettings} className="space-y-4">
                  <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
                    <Field>
                      <FieldLabel htmlFor="app-edit-name">App Name</FieldLabel>
                      <Input
                        id="app-edit-name"
                        type="text"
                        required
                        value={editName}
                        onChange={(e) => setEditName(e.target.value)}
                      />
                    </Field>
                    <Field>
                      <FieldLabel htmlFor="app-edit-port">Container Port</FieldLabel>
                      <Input
                        id="app-edit-port"
                        type="number"
                        min={1}
                        max={65535}
                        value={editPort}
                        onChange={(e) => setEditPort(Number(e.target.value))}
                        className="font-mono"
                      />
                    </Field>
                    <Field>
                      <FieldLabel htmlFor="app-edit-mem">Memory Limit (MB)</FieldLabel>
                      <Input
                        id="app-edit-mem"
                        type="number"
                        min={64}
                        max={65536}
                        value={editMemoryMb}
                        onChange={(e) => setEditMemoryMb(Number(e.target.value))}
                        className="font-mono"
                      />
                    </Field>
                    <Field>
                      <FieldLabel htmlFor="app-edit-health">Health Check Path</FieldLabel>
                      <Input
                        id="app-edit-health"
                        type="text"
                        value={editHealthPath}
                        onChange={(e) => setEditHealthPath(e.target.value)}
                        placeholder="/healthz"
                        className="font-mono"
                      />
                    </Field>
                  </div>

                  {app.source_kind === 'image' && (
                    <Field>
                      <FieldLabel htmlFor="app-edit-image">Container Image</FieldLabel>
                      <Input
                        id="app-edit-image"
                        type="text"
                        value={editImage}
                        onChange={(e) => setEditImage(e.target.value)}
                        className="font-mono"
                      />
                    </Field>
                  )}

                  {app.source_kind === 'git' && (
                    <div className="grid grid-cols-1 gap-4 sm:grid-cols-3">
                      <Field className="sm:col-span-2">
                        <FieldLabel htmlFor="app-edit-git-url">Git Repository URL</FieldLabel>
                        <Input
                          id="app-edit-git-url"
                          type="text"
                          value={editGitUrl}
                          onChange={(e) => setEditGitUrl(e.target.value)}
                          className="font-mono"
                        />
                      </Field>
                      <Field>
                        <FieldLabel htmlFor="app-edit-git-branch">Branch</FieldLabel>
                        {appBranchesQuery.isSuccess && appBranchItems.length > 0 ? (
                          <Combobox
                            items={appBranchItems}
                            value={
                              appBranchItems.some((i) => i.value === editGitBranch)
                                ? editGitBranch
                                : ''
                            }
                            onValueChange={(v) => {
                              if (v) setEditGitBranch(String(v))
                            }}
                          >
                            <ComboboxInput
                              id="app-edit-git-branch"
                              placeholder="Select branch…"
                            />
                            <ComboboxContent>
                              <ComboboxEmpty>No branch matches.</ComboboxEmpty>
                              <ComboboxList>
                                {(item) => {
                                  const b = (item as (typeof appBranchItems)[number]).branch
                                  return (
                                    <ComboboxItem key={b.name} value={b.name}>
                                      <span className="flex w-full min-w-0 items-center gap-2">
                                        <GitBranch className="size-3.5 shrink-0 text-muted-foreground" />
                                        <span className="min-w-0 flex-1 truncate font-mono text-[13px]">
                                          {b.name}
                                        </span>
                                        {b.default && (
                                          <Badge variant="solid" color="gray" size="compact">
                                            default
                                          </Badge>
                                        )}
                                        {b.protected && (
                                          <ShieldCheck className="size-3.5 shrink-0 text-muted-foreground" />
                                        )}
                                      </span>
                                    </ComboboxItem>
                                  )
                                }}
                              </ComboboxList>
                            </ComboboxContent>
                          </Combobox>
                        ) : (
                          <Input
                            id="app-edit-git-branch"
                            type="text"
                            value={editGitBranch}
                            onChange={(e) => setEditGitBranch(e.target.value)}
                            className="font-mono"
                          />
                        )}
                      </Field>
                    </div>
                  )}

                  {app.source_kind !== 'image' && (
                    <Field>
                      <FieldLabel htmlFor="app-edit-dockerfile">
                        Dockerfile Path <span className="opacity-70">(empty = Railpack)</span>
                      </FieldLabel>
                      <Input
                        id="app-edit-dockerfile"
                        type="text"
                        value={editDockerfile}
                        onChange={(e) => setEditDockerfile(e.target.value)}
                        placeholder="Dockerfile"
                        className="font-mono"
                      />
                    </Field>
                  )}

                  {app.source_kind === 'git' && (
                    <Switch
                      label="Auto-deploy on Git push webhook"
                      checked={editAutoDeploy}
                      onToggle={() => setEditAutoDeploy((v) => !v)}
                    />
                  )}

                  <div className="flex justify-end border-t border-border/80 pt-4">
                    <Button
                      type="submit"
                      variant="primary"
                      leadingIcon={Save}
                      loading={savingSettings}
                    >
                      Save Changes
                    </Button>
                  </div>
                </form>
              </Panel>

              <Panel
                tone="danger"
                title="Delete Application"
                description={`Stops containers and permanently removes ${app.name}, its deployments, and domains.`}
                actions={
                  <Button
                    type="button"
                    variant="destructive"
                    leadingIcon={Trash2}
                    disabled={savingSettings}
                    onClick={() =>
                      setConfirmState({
                        title: 'Delete Application',
                        description:
                          'This permanently stops the container and deletes all deployments, environment variables, and domains for this app. This cannot be undone.',
                        target: app.name,
                        confirmLabel: 'Delete App',
                        tone: 'danger',
                        action: handleDeleteApp,
                      })
                    }
                  >
                    Delete App
                  </Button>
                }
              >
                <p className="text-[13px] text-muted-foreground">
                  This action cannot be undone.
                </p>
              </Panel>
            </div>
          )}
        </div>

        <ConfirmDialog
          open={Boolean(confirmState)}
          onOpenChange={(open) => {
            if (!open) setConfirmState(null)
          }}
          title={confirmState?.title ?? ''}
          description={confirmState?.description ?? ''}
          target={confirmState?.target}
          confirmLabel={confirmState?.confirmLabel}
          tone={confirmState?.tone}
          onConfirm={async () => {
            await confirmState?.action()
          }}
        />
      </div>
    )
  }

  const query = appSearch.trim().toLowerCase()
  const filteredApps = apps.filter((a) => {
    if (projectFilter !== 'all' && a.project_id !== projectFilter) return false
    if (!query) return true
    return (
      a.name.toLowerCase().includes(query) ||
      a.project_name.toLowerCase().includes(query) ||
      a.source_kind.toLowerCase().includes(query) ||
      a.domains.some((d) => d.toLowerCase().includes(query))
    )
  })

  const projectTabs = [
    { id: 'all', label: `All (${apps.length})` },
    ...projects.map((p) => ({ id: p.id, label: `${p.name} (${p.app_count})` })),
  ]
  const selectedProjectIdx = Math.max(
    0,
    projectTabs.findIndex((t) => t.id === projectFilter),
  )

  return (
    <Page
      title="Applications"
      description="Manage deployed containers, inspect live logs, sync .env secrets, and attach domains."
      actions={
        <>
          <Button
            type="button"
            variant="tertiary"
            leadingIcon={FolderPlus}
            onClick={() => setNewProjectOpen((v) => !v)}
          >
            New Project
          </Button>
          <Button type="button" variant="primary" leadingIcon={Rocket} onClick={onDeployNew}>
            Deploy App
          </Button>
        </>
      }
    >
      {newProjectOpen && (
        <Panel>
          <form onSubmit={handleCreateProject} className="flex flex-wrap items-center gap-2.5">
            <Input
              type="text"
              required
              value={newProjectName}
              onChange={(e) => setNewProjectName(e.target.value)}
              placeholder="Project name (e.g. Production, Homelab)"
              className="w-64"
              autoFocus
            />
            <Input
              type="text"
              value={newProjectDesc}
              onChange={(e) => setNewProjectDesc(e.target.value)}
              placeholder="Description (optional)"
              className="min-w-[200px] flex-1"
            />
            <Button
              type="submit"
              variant="primary"
              leadingIcon={Plus}
              loading={creatingProject}
              disabled={!newProjectName.trim()}
            >
              Create Project
            </Button>
            <Button
              type="button"
              variant="ghost"
              size="icon"
              onClick={() => setNewProjectOpen(false)}
              aria-label="Close new project form"
            >
              <X />
            </Button>
          </form>
        </Panel>
      )}

      {/* Fluid Project Filter Tabs + Search */}
      {apps.length > 0 && (
        <div className="flex flex-wrap items-center justify-between gap-3 border-b border-border/80 pb-3">
          {projectTabs.length > 1 ? (
            <Tabs
              size="compact"
              selectedIndex={selectedProjectIdx}
              onSelect={(idx) => {
                const target = projectTabs[idx]
                if (target) setProjectFilter(target.id)
              }}
            >
              {projectTabs.map((t, idx) => (
                <TabsItem key={t.id} index={idx} label={t.label} />
              ))}
            </Tabs>
          ) : (
            <span className="text-[13px] text-muted-foreground">
              {apps.length} {apps.length === 1 ? 'application' : 'applications'}
            </span>
          )}
          <div className="relative w-full sm:w-64">
            <Search className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground" />
            <Input
              size="compact"
              type="text"
              value={appSearch}
              onChange={(e) => setAppSearch(e.target.value)}
              placeholder="Filter apps or domains…"
              className="pl-8"
            />
          </div>
        </div>
      )}

      {filteredApps.length === 0 ? (
        <EmptyState
          icon={Rocket}
          title="No applications yet"
          action={
            <Button type="button" variant="primary" leadingIcon={Plus} onClick={onDeployNew}>
              Deploy Your First App
            </Button>
          }
        >
          Deploy a local repository from this machine, a Docker image, or a Git repository.
        </EmptyState>
      ) : (
        <CardGroup columns={2} border="outlined" separated className="grid-cols-1 md:grid-cols-2 gap-3.5">
          {filteredApps.map((app) => (
            <Card
              key={app.id}
              onClick={() => onSelectApp(app.id)}
              label={`Open ${app.name}`}
              className="bg-card shadow-[var(--shadow-2)]"
            >
              <CardHeader>
                <div className="flex items-center justify-between gap-2">
                  <CardTitle className="truncate font-semibold">{app.name}</CardTitle>
                  <Status tone={statusTone(app.status)} size="compact" variant="dot">
                    {app.status}
                  </Status>
                </div>
                <div className="font-mono text-[12px] text-muted-foreground">
                  {app.project_name} · {app.source_kind} · port {app.port}
                  {app.git_branch ? ` · ${app.git_branch}` : ''}
                </div>
              </CardHeader>

              {(app.domains.length > 0 || app.last_crash?.reason) && (
                <CardContent className="space-y-2">
                  {app.domains.length > 0 && (
                    <div className="flex flex-wrap gap-1.5">
                      {app.domains.slice(0, 2).map((d) => (
                        <Badge key={d} variant="solid" color="emerald" size="compact" className="font-mono">
                          {d}
                        </Badge>
                      ))}
                    </div>
                  )}
                  {app.last_crash?.reason && (
                    <Banner status="error" contrast="high" size="compact">
                      <BannerDescription className="truncate text-[12px]">
                        {app.last_crash.reason}
                      </BannerDescription>
                    </Banner>
                  )}
                </CardContent>
              )}

              <CardFooter className="mt-auto border-t border-border/70 pt-3 gap-1.5">
                <Button
                  type="button"
                  variant="primary"
                  size="compact"
                  leadingIcon={Play}
                  loading={busyActionId === `${app.id}:deploy`}
                  disabled={busyActionId !== null}
                  onClick={() => {
                    if (app.source_kind === 'local') {
                      onSelectApp(app.id)
                      onSelectTab('local-git')
                    } else {
                      void runAction(app.id, app.name, 'deploy')
                    }
                  }}
                >
                  Deploy
                </Button>
                <Button
                  type="button"
                  variant="tertiary"
                  size="compact"
                  leadingIcon={RotateCcw}
                  loading={busyActionId === `${app.id}:restart`}
                  disabled={busyActionId !== null}
                  onClick={() => void runAction(app.id, app.name, 'restart')}
                >
                  Restart
                </Button>
                <Button
                  type="button"
                  variant="ghost"
                  size="compact"
                  leadingIcon={ScrollText}
                  onClick={() => {
                    onSelectApp(app.id)
                    onSelectTab('logs')
                  }}
                >
                  Logs
                </Button>
                <Button
                  type="button"
                  variant="ghost"
                  size="compact"
                  leadingIcon={KeyRound}
                  onClick={() => {
                    onSelectApp(app.id)
                    onSelectTab('env')
                  }}
                >
                  Env
                </Button>
                {app.status === 'running' && (
                  <Button
                    type="button"
                    variant="secondary"
                    size="compact"
                    leadingIcon={Terminal}
                    className="ml-auto"
                    onClick={() => {
                      onSelectApp(app.id)
                      onSelectTab('terminal')
                    }}
                  >
                    Shell
                  </Button>
                )}
              </CardFooter>
            </Card>
          ))}
        </CardGroup>
      )}
    </Page>
  )
}
