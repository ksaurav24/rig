import { useQuery, useQueryClient } from '@tanstack/react-query'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import {
  Box,
  ExternalLink,
  FolderGit2,
  GitBranch,
  Globe,
  HardDrive,
  Link2,
  Lock,
  Package,
  Plus,
  RefreshCw,
  Rocket,
  Search,
  ShieldCheck,
  X,
} from 'lucide-react'
import { useEffect, useMemo, useState, type FormEvent, type KeyboardEvent } from 'react'
import { toast } from 'sonner'
import { GithubConnectPanel } from './GithubConnectPanel'
import { Badge } from '@rig/ui/badge'
import { Banner, BannerDescription, BannerTitle } from '@rig/ui/banner'
import { Button } from '@rig/ui/button'
import {
  Combobox,
  ComboboxContent,
  ComboboxEmpty,
  ComboboxInput,
  ComboboxItem,
  ComboboxList,
} from '@rig/ui/combobox'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@rig/ui/dialog'
import { Field, FieldDescription, FieldLabel } from '@rig/ui/field'
import { Input } from '@rig/ui/input'
import { Select, SelectContent, SelectItem, SelectTrigger } from '@rig/ui/select'
import { Switch } from '@rig/ui/switch'
import { Tabs, TabsItem } from '@rig/ui/tabs'
import { suggestAppName } from '@/lib/format'
import { desktopApi } from '@/lib/ipc'
import type { App, DiscoveredRepo, GithubRepo, Project, SourceKind } from '@/lib/types'
import { cn } from '@rig/ui/lib/utils'

interface DeployModalProps {
  open: boolean
  serverId: string
  serverUrl?: string
  projects: Project[]
  initialLocalPath?: string | null
  initialName?: string | null
  onClose: () => void
  onDeployed: (app: App) => void
}

type DeploySourceMode = 'github' | 'local' | 'image' | 'git-url'
const SOURCE_MODES: DeploySourceMode[] = ['github', 'local', 'image', 'git-url']

/** Prevents Enter inside a Combobox input from prematurely submitting the enclosing form. */
function keepComboboxEnter(e: KeyboardEvent<HTMLFormElement>) {
  if (e.key !== 'Enter') return
  const target = e.target as HTMLElement | null
  if (target?.getAttribute('role') === 'combobox') {
    e.preventDefault()
  }
}

export function DeployModal({
  open,
  serverId,
  serverUrl,
  projects,
  initialLocalPath,
  initialName,
  onClose,
  onDeployed,
}: DeployModalProps) {
  const qc = useQueryClient()
  const [sourceMode, setSourceMode] = useState<DeploySourceMode>('github')
  const [projectId, setProjectId] = useState<string>(projects[0]?.id ?? '__new__')
  const [newProjectName, setNewProjectName] = useState('Default')
  const [appName, setAppName] = useState('')
  const [nameTouched, setNameTouched] = useState(false)
  const [port, setPort] = useState(3000)
  const [memoryMb, setMemoryMb] = useState(512)
  const [cpuCores, setCpuCores] = useState('')
  const [volumes, setVolumes] = useState<string[]>([])
  const [volumeInput, setVolumeInput] = useState('')
  const [dockerfilePath, setDockerfilePath] = useState('Dockerfile')

  // GitHub connected account state
  const [selectedRepo, setSelectedRepo] = useState<GithubRepo | null>(null)
  const [githubBranch, setGithubBranch] = useState('')
  const [autoDeploy, setAutoDeploy] = useState(true)

  // Manual image / Git URL state
  const [image, setImage] = useState('nginx:alpine')
  const [gitUrl, setGitUrl] = useState('')
  const [gitBranch, setGitBranch] = useState('main')

  // Local folder state
  const [localPath, setLocalPath] = useState('')
  const [localRepos, setLocalRepos] = useState<DiscoveredRepo[]>([])
  const [repoSearch, setRepoSearch] = useState('')
  const [scanning, setScanning] = useState(false)

  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const githubStatusQuery = useQuery({
    queryKey: ['desktop', serverId, 'github-status'],
    queryFn: () => desktopApi.getGithubStatus(serverId),
    enabled: open && sourceMode === 'github',
  })

  const ghApp = githubStatusQuery.data?.app ?? null

  const githubReposQuery = useQuery({
    queryKey: ['desktop', serverId, 'github-repos'],
    queryFn: () => desktopApi.listGithubRepos(serverId),
    enabled: open && sourceMode === 'github' && Boolean(ghApp),
  })

  const [repoOwner, repoName] = selectedRepo ? selectedRepo.full_name.split('/') : ['', '']

  const githubBranchesQuery = useQuery({
    queryKey: ['desktop', serverId, 'github-branches', selectedRepo?.full_name],
    queryFn: () => desktopApi.listGithubBranches(serverId, repoOwner, repoName),
    enabled: open && sourceMode === 'github' && Boolean(repoOwner && repoName),
  })

  useEffect(() => {
    if (!open) return
    setError(null)
    setProjectId(projects[0]?.id ?? '__new__')
    setSelectedRepo(null)
    setGithubBranch('')
    setAutoDeploy(true)
    setCpuCores('')
    setVolumes([])
    setVolumeInput('')
    setNameTouched(false)
    if (initialLocalPath) {
      setSourceMode('local')
      setLocalPath(initialLocalPath)
      const folderName =
        suggestAppName(initialName || initialLocalPath) || 'my-app'
      setAppName(folderName)
    } else {
      setSourceMode('github')
      setAppName('')
    }
    setScanning(true)
    void desktopApi
      .discoverLocalRepos()
      .then(setLocalRepos)
      .catch(() => {})
      .finally(() => setScanning(false))
  }, [initialLocalPath, initialName, open, projects])

  const filteredLocalRepos = useMemo(() => {
    const q = repoSearch.trim().toLowerCase()
    if (!q) return localRepos
    return localRepos.filter(
      (r) =>
        r.name.toLowerCase().includes(q) ||
        r.path.toLowerCase().includes(q) ||
        r.stacks.some((s) => s.toLowerCase().includes(q)),
    )
  }, [repoSearch, localRepos])

  const githubRepoItems = useMemo(
    () =>
      (githubReposQuery.data ?? []).map((r) => ({
        value: r.full_name,
        label: r.full_name,
        repo: r,
      })),
    [githubReposQuery.data],
  )

  const githubBranchItems = useMemo(
    () =>
      (githubBranchesQuery.data ?? []).map((b) => ({
        value: b.name,
        label: b.name,
        branch: b,
      })),
    [githubBranchesQuery.data],
  )

  function autoFillName(raw: string) {
    if (!nameTouched) {
      const suggested = suggestAppName(raw)
      if (suggested) setAppName(suggested)
    }
  }

  async function pickFolder() {
    const selected = await openDialog({
      directory: true,
      multiple: false,
      title: 'Select local project folder to deploy',
    })
    if (typeof selected === 'string') {
      setLocalPath(selected)
      autoFillName(selected)
      try {
        const info = await desktopApi.inspectLocalGit(selected)
        setDockerfilePath(info.has_dockerfile ? 'Dockerfile' : '')
      } catch {
        // ignore
      }
    }
  }

  const githubMissing = sourceMode === 'github' && githubStatusQuery.isSuccess && !ghApp

  async function handleSubmit(e: FormEvent) {
    e.preventDefault()
    setError(null)
    if (!appName.trim()) {
      setError('Enter an app name')
      return
    }
    if (sourceMode === 'github' && !selectedRepo) {
      setError('Choose a repository from your connected GitHub account')
      return
    }
    if (sourceMode === 'local' && !localPath.trim()) {
      setError('Choose or enter a local folder to deploy')
      return
    }
    if (sourceMode === 'image' && !image.trim()) {
      setError('Enter a Docker image (e.g. nginx:alpine)')
      return
    }
    if (sourceMode === 'git-url' && !gitUrl.trim()) {
      setError('Enter a Git repository URL')
      return
    }

    setBusy(true)
    try {
      let targetProjectId = projectId
      if (targetProjectId === '__new__') {
        const proj = await desktopApi.createProject(
          serverId,
          newProjectName.trim() || 'Default',
          '',
        )
        targetProjectId = proj.id
      }

      const apiSourceKind: SourceKind =
        sourceMode === 'github' || sourceMode === 'git-url'
          ? 'git'
          : sourceMode

      const resolvedGitUrl =
        sourceMode === 'github'
          ? selectedRepo?.clone_url ?? null
          : sourceMode === 'git-url'
            ? gitUrl.trim()
            : null

      const resolvedGitBranch =
        sourceMode === 'github'
          ? githubBranch.trim() || selectedRepo?.default_branch || null
          : sourceMode === 'git-url'
            ? gitBranch.trim() || 'main'
            : null

      const parsedCpu = cpuCores.trim() ? Number(cpuCores) : NaN
      const cpuMillis =
        Number.isFinite(parsedCpu) && parsedCpu > 0 ? Math.round(parsedCpu * 1000) : null

      let created = await desktopApi.createApp(serverId, {
        project_id: targetProjectId,
        name: appName.trim(),
        source_kind: apiSourceKind,
        image: sourceMode === 'image' ? image.trim() : null,
        git_url: resolvedGitUrl,
        git_branch: resolvedGitBranch,
        dockerfile_path: sourceMode === 'image' ? null : dockerfilePath.trim() || 'Dockerfile',
        port: Number(port) || 3000,
        memory_mb: Number(memoryMb) || 512,
        cpu_millis: cpuMillis,
        volumes: volumes.length > 0 ? volumes : undefined,
      })

      if (sourceMode === 'github' && autoDeploy) {
        created = await desktopApi
          .updateApp(serverId, created.id, { auto_deploy: true })
          .catch(() => created)
      }

      if (sourceMode === 'local') {
        await desktopApi.deployLocalGit(
          serverId,
          created.id,
          localPath.trim(),
          dockerfilePath.trim() || undefined,
        )
        toast.success(`Uploaded & deploying ${created.name}`)
      } else {
        await desktopApi.appAction(serverId, created.id, 'deploy')
        toast.success(`Created & deploying ${created.name}`)
      }

      onDeployed(created)
      onClose()
    } catch (err) {
      setError(String(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) {
          void desktopApi.cancelGithubConnect()
          onClose()
        }
      }}
    >
      <DialogContent size="xl" className="flex max-h-[90vh] max-w-2xl flex-col overflow-hidden p-0">
        <DialogHeader className="mb-0 border-b border-border/80 px-6 py-4 pr-12">
          <DialogTitle className="flex items-center gap-2.5">
            <Rocket className="size-4.5 text-primary" />
            Deploy New Application
          </DialogTitle>
          <DialogDescription>
            Deploy from a connected GitHub repository, a local folder, a Docker image, or a Git URL.
          </DialogDescription>
        </DialogHeader>

        <form
          id="deploy-app-form"
          onSubmit={handleSubmit}
          onKeyDown={keepComboboxEnter}
          className="min-h-0 flex-1 space-y-4 overflow-y-auto px-6 py-5"
        >
          {error && (
            <Banner role="alert" status="error" contrast="high" size="compact">
              <BannerDescription>{error}</BannerDescription>
            </Banner>
          )}

          {/* Full-width 4-column Fluid Source Switcher */}
          <div className="rounded-xl border border-border/80 bg-surface p-1">
            <Tabs
              className="mx-0 my-0 w-full max-w-full px-0 py-0"
              selectedIndex={SOURCE_MODES.indexOf(sourceMode)}
              onSelect={(idx) => {
                setSourceMode(SOURCE_MODES[idx] ?? 'github')
                setError(null)
              }}
            >
              <TabsItem
                index={0}
                icon={GitBranch}
                label="GitHub"
                className="flex-1 justify-center"
              />
              <TabsItem
                index={1}
                icon={FolderGit2}
                label="Local Repo"
                className="flex-1 justify-center"
              />
              <TabsItem
                index={2}
                icon={Package}
                label="Docker Image"
                className="flex-1 justify-center"
              />
              <TabsItem
                index={3}
                icon={Link2}
                label="Git URL"
                className="flex-1 justify-center"
              />
            </Tabs>
          </div>

          {/* GITHUB CONNECTED ACCOUNT SOURCE */}
          {sourceMode === 'github' && (
            <div className="space-y-3.5">
              {githubStatusQuery.isPending ? (
                <div className="rounded-xl border border-border/80 bg-surface/60 px-4 py-6 text-center text-[13px] text-muted-foreground">
                  Checking GitHub connection on this server…
                </div>
              ) : githubStatusQuery.isError ? (
                <Banner status="error" contrast="high" size="compact">
                  <BannerTitle>Could not reach GitHub settings</BannerTitle>
                  <BannerDescription>{String(githubStatusQuery.error)}</BannerDescription>
                </Banner>
              ) : !ghApp ? (
                <GithubConnectPanel
                  serverId={serverId}
                  serverUrl={serverUrl}
                  compact
                  onConnected={(status) => {
                    qc.setQueryData(['desktop', serverId, 'github-status'], status)
                    void qc.invalidateQueries({ queryKey: ['desktop', serverId, 'settings'] })
                    void qc.invalidateQueries({ queryKey: ['desktop', serverId, 'github-repos'] })
                  }}
                  onUseGitUrl={() => setSourceMode('git-url')}
                />
              ) : (
                <div className="space-y-3.5 rounded-xl border border-border/80 bg-surface/60 p-4">
                  {/* Connected GitHub Account Header */}
                  <div className="flex flex-wrap items-center justify-between gap-2.5 rounded-lg border border-border/80 bg-card px-3.5 py-2.5">
                    <div className="flex min-w-0 items-center gap-2.5">
                      <div className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-primary/12 text-primary">
                        <GitBranch className="size-4" />
                      </div>
                      <div className="min-w-0 leading-tight">
                        <div className="truncate text-[13.5px] font-medium text-foreground">
                          {ghApp.owner}
                        </div>
                        <div className="truncate text-[12px] text-muted-foreground">
                          Connected via {ghApp.name}
                        </div>
                      </div>
                    </div>
                    <div className="flex shrink-0 items-center gap-1.5">
                      <Button
                        type="button"
                        variant="ghost"
                        size="compact"
                        leadingIcon={RefreshCw}
                        loading={githubReposQuery.isFetching}
                        onClick={() => void githubReposQuery.refetch()}
                      >
                        Refresh
                      </Button>
                      <Button
                        type="button"
                        variant="tertiary"
                        size="compact"
                        trailingIcon={ExternalLink}
                        onClick={() => void desktopApi.openExternalUrl(ghApp.install_url)}
                      >
                        Configure access
                      </Button>
                    </div>
                  </div>

                  {/* Repository Picker */}
                  <Field>
                    <FieldLabel htmlFor="deploy-gh-repo">Repository</FieldLabel>
                    {githubReposQuery.isPending ? (
                      <div className="rounded-lg border border-border bg-card px-3.5 py-2.5 text-[13px] text-muted-foreground">
                        Loading repositories from GitHub…
                      </div>
                    ) : githubReposQuery.isError ? (
                      <Banner status="error" contrast="high" size="compact">
                        <BannerDescription>{String(githubReposQuery.error)}</BannerDescription>
                      </Banner>
                    ) : githubRepoItems.length === 0 ? (
                      <Banner status="info" contrast="low" size="compact">
                        <BannerTitle>No repositories granted yet</BannerTitle>
                        <BannerDescription>
                          Click &ldquo;Configure access&rdquo; above to install{' '}
                          <span className="font-medium text-foreground">{ghApp.name}</span> on the
                          repositories you want to deploy, then click Refresh.
                        </BannerDescription>
                      </Banner>
                    ) : (
                      <Combobox
                        items={githubRepoItems}
                        value={selectedRepo?.full_name ?? ''}
                        onValueChange={(v) => {
                          const found =
                            githubReposQuery.data?.find((r) => r.full_name === v) ?? null
                          setSelectedRepo(found)
                          setGithubBranch(found?.default_branch ?? '')
                          if (found) autoFillName(found.full_name)
                        }}
                      >
                        <ComboboxInput
                          id="deploy-gh-repo"
                          placeholder={`Search ${githubRepoItems.length} repositories…`}
                          clearable
                        />
                        <ComboboxContent>
                          <ComboboxEmpty>
                            No repository matches. Missing one? Use &ldquo;Configure access&rdquo;.
                          </ComboboxEmpty>
                          <ComboboxList>
                            {(item) => {
                              const r = (item as (typeof githubRepoItems)[number]).repo
                              const [owner, name] = r.full_name.split('/')
                              return (
                                <ComboboxItem key={r.full_name} value={r.full_name}>
                                  <span className="flex w-full min-w-0 items-center gap-2">
                                    <span className="min-w-0 flex-1 truncate">
                                      <span className="text-muted-foreground">{owner}/</span>
                                      <span className="font-medium">{name}</span>
                                    </span>
                                    <span className="shrink-0 font-mono text-[11.5px] text-muted-foreground">
                                      {r.default_branch}
                                    </span>
                                    {r.private ? (
                                      <Lock
                                        className="size-3.5 shrink-0 text-muted-foreground"
                                        aria-label="Private"
                                      />
                                    ) : (
                                      <Globe
                                        className="size-3.5 shrink-0 text-muted-foreground/60"
                                        aria-label="Public"
                                      />
                                    )}
                                  </span>
                                </ComboboxItem>
                              )
                            }}
                          </ComboboxList>
                        </ComboboxContent>
                      </Combobox>
                    )}
                  </Field>

                  {/* Branch Picker */}
                  <Field>
                    <FieldLabel htmlFor="deploy-gh-branch">Branch</FieldLabel>
                    {!selectedRepo ? (
                      <Input
                        id="deploy-gh-branch"
                        disabled
                        value=""
                        placeholder="Pick a repository first"
                        className="font-mono"
                      />
                    ) : githubBranchesQuery.isPending ? (
                      <div className="rounded-lg border border-border bg-card px-3.5 py-2.5 text-[13px] text-muted-foreground">
                        Loading branches…
                      </div>
                    ) : githubBranchesQuery.isSuccess && githubBranchItems.length > 0 ? (
                      <Combobox
                        items={githubBranchItems}
                        value={
                          githubBranchItems.some((i) => i.value === githubBranch)
                            ? githubBranch
                            : ''
                        }
                        onValueChange={(v) => {
                          if (v) setGithubBranch(String(v))
                        }}
                      >
                        <ComboboxInput
                          id="deploy-gh-branch"
                          placeholder={`Search ${githubBranchItems.length} branch${githubBranchItems.length === 1 ? '' : 'es'}…`}
                        />
                        <ComboboxContent>
                          <ComboboxEmpty>No branch matches.</ComboboxEmpty>
                          <ComboboxList>
                            {(item) => {
                              const b = (item as (typeof githubBranchItems)[number]).branch
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
                                      <ShieldCheck
                                        className="size-3.5 shrink-0 text-muted-foreground"
                                        aria-label="Protected"
                                      />
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
                        id="deploy-gh-branch"
                        type="text"
                        value={githubBranch}
                        onChange={(e) => setGithubBranch(e.target.value)}
                        placeholder={selectedRepo.default_branch || 'main'}
                        className="font-mono"
                      />
                    )}
                  </Field>

                  {/* Deploy on Push toggle */}
                  <div className="flex items-center justify-between gap-4 rounded-lg border border-border/80 bg-card px-3.5 py-2.5">
                    <div className="min-w-0">
                      <div className="text-[13px] font-medium text-foreground">Deploy on push</div>
                      <div className="text-[12px] text-muted-foreground">
                        Automatically redeploy when{' '}
                        <code className="font-mono text-foreground">
                          {githubBranch.trim() || selectedRepo?.default_branch || 'the branch'}
                        </code>{' '}
                        receives new commits.
                      </div>
                    </div>
                    <Switch
                      checked={autoDeploy}
                      onToggle={() => setAutoDeploy((v) => !v)}
                      aria-label="Deploy on push"
                    />
                  </div>
                </div>
              )}
            </div>
          )}

          {/* LOCAL FOLDER / REPO SOURCE */}
          {sourceMode === 'local' && (
            <div className="space-y-3 rounded-xl border border-border/80 bg-surface/60 p-4">
              <div className="flex items-center justify-between gap-2">
                <span className="text-[13.5px] font-medium text-foreground">
                  Local project directory
                </span>
                <Button
                  type="button"
                  variant="secondary"
                  size="compact"
                  leadingIcon={FolderGit2}
                  onClick={pickFolder}
                >
                  Browse Folder…
                </Button>
              </div>
              <Input
                type="text"
                value={localPath}
                onChange={(e) => setLocalPath(e.target.value)}
                placeholder="/home/you/code/my-app"
                className="font-mono"
              />

              <div className="pt-1">
                <div className="mb-2 flex items-center justify-between gap-2">
                  <span className="text-[12px] font-medium text-muted-foreground">
                    Discovered local repos ({filteredLocalRepos.length})
                  </span>
                  <div className="flex items-center gap-1.5">
                    <div className="relative">
                      <Search className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground" />
                      <Input
                        size="compact"
                        type="text"
                        value={repoSearch}
                        onChange={(e) => setRepoSearch(e.target.value)}
                        placeholder="Filter repos…"
                        className="w-44 pl-8"
                      />
                    </div>
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon-compact"
                      onClick={() => {
                        setScanning(true)
                        void desktopApi
                          .discoverLocalRepos()
                          .then(setLocalRepos)
                          .finally(() => setScanning(false))
                      }}
                      title="Rescan local repos"
                      aria-label="Rescan local repos"
                    >
                      <RefreshCw className={cn('size-3.5', scanning && 'animate-spin')} />
                    </Button>
                  </div>
                </div>
                <div className="max-h-40 divide-y divide-border/80 overflow-y-auto rounded-lg border border-border bg-card">
                  {filteredLocalRepos.length === 0 ? (
                    <div className="px-4 py-5 text-center text-[12.5px] text-muted-foreground">
                      {scanning
                        ? 'Scanning ~/code, ~/projects, ~/src…'
                        : 'No repos matched — use Browse Folder above.'}
                    </div>
                  ) : (
                    filteredLocalRepos.map((r) => {
                      const active = localPath === r.path
                      return (
                        <button
                          key={r.path}
                          type="button"
                          onClick={() => {
                            setLocalPath(r.path)
                            autoFillName(r.name)
                            setDockerfilePath(r.has_dockerfile ? 'Dockerfile' : '')
                          }}
                          className={cn(
                            'flex w-full items-center justify-between gap-2 px-3.5 py-2.5 text-left text-[13px] transition-colors',
                            active ? 'bg-active text-foreground' : 'hover:bg-hover',
                          )}
                        >
                          <div className="min-w-0">
                            <div className="flex items-center gap-2 font-medium text-foreground">
                              <span className="truncate">{r.name}</span>
                              {r.branch && (
                                <Badge variant="solid" color="gray" size="compact" className="font-mono">
                                  {r.branch}
                                </Badge>
                              )}
                            </div>
                            <div className="truncate font-mono text-[11.5px] text-muted-foreground">
                              {r.path}
                            </div>
                          </div>
                          <div className="flex shrink-0 items-center gap-1">
                            {r.stacks.slice(0, 2).map((s) => (
                              <Badge key={s} variant="solid" color="emerald" size="compact">
                                {s}
                              </Badge>
                            ))}
                          </div>
                        </button>
                      )
                    })
                  )}
                </div>
              </div>
            </div>
          )}

          {/* DOCKER IMAGE SOURCE */}
          {sourceMode === 'image' && (
            <Field>
              <FieldLabel htmlFor="deploy-image">Container Image</FieldLabel>
              <Input
                id="deploy-image"
                type="text"
                required
                value={image}
                onChange={(e) => {
                  setImage(e.target.value)
                  autoFillName(e.target.value)
                }}
                placeholder="nginx:alpine or ghcr.io/org/service:latest"
                className="font-mono"
              />
            </Field>
          )}

          {/* MANUAL GIT URL SOURCE */}
          {sourceMode === 'git-url' && (
            <div className="grid grid-cols-1 gap-3.5 sm:grid-cols-3">
              <Field className="sm:col-span-2">
                <FieldLabel htmlFor="deploy-git-url">Git Repository URL</FieldLabel>
                <Input
                  id="deploy-git-url"
                  type="text"
                  required
                  value={gitUrl}
                  onChange={(e) => {
                    setGitUrl(e.target.value)
                    autoFillName(e.target.value)
                  }}
                  placeholder="https://github.com/owner/repo.git"
                  className="font-mono"
                />
                <FieldDescription>
                  Public repositories only. Use the GitHub tab for private repositories.
                </FieldDescription>
              </Field>
              <Field>
                <FieldLabel htmlFor="deploy-git-branch">Branch</FieldLabel>
                <Input
                  id="deploy-git-branch"
                  type="text"
                  value={gitBranch}
                  onChange={(e) => setGitBranch(e.target.value)}
                  placeholder="main"
                  className="font-mono"
                />
              </Field>
            </div>
          )}

          {/* Core App Metadata (hidden while GitHub isn't connected yet on GitHub tab) */}
          {!githubMissing && (
            <>
              <div className="grid grid-cols-1 gap-3.5 sm:grid-cols-2">
                <Field>
                  <FieldLabel>Project</FieldLabel>
                  <Select value={projectId} onValueChange={setProjectId}>
                    <SelectTrigger className="w-full" placeholder="Select project" />
                    <SelectContent>
                      {projects.map((p, idx) => (
                        <SelectItem key={p.id} index={idx} value={p.id}>
                          {p.name}
                        </SelectItem>
                      ))}
                      <SelectItem index={projects.length} value="__new__">
                        + Create new project…
                      </SelectItem>
                    </SelectContent>
                  </Select>
                </Field>
                <Field>
                  <FieldLabel htmlFor="deploy-app-name">App Name</FieldLabel>
                  <Input
                    id="deploy-app-name"
                    type="text"
                    required
                    value={appName}
                    onChange={(e) => {
                      setAppName(e.target.value)
                      setNameTouched(true)
                    }}
                    placeholder="api, web, worker…"
                  />
                </Field>
              </div>

              {projectId === '__new__' && (
                <Field>
                  <FieldLabel htmlFor="deploy-new-proj">New Project Name</FieldLabel>
                  <Input
                    id="deploy-new-proj"
                    type="text"
                    required
                    value={newProjectName}
                    onChange={(e) => setNewProjectName(e.target.value)}
                    placeholder="Production, Homelab…"
                  />
                </Field>
              )}

              <div className="grid grid-cols-1 gap-3.5 sm:grid-cols-4">
                <Field>
                  <FieldLabel htmlFor="deploy-port">Container Port</FieldLabel>
                  <Input
                    id="deploy-port"
                    type="number"
                    min={1}
                    max={65535}
                    value={port}
                    onChange={(e) => setPort(Number(e.target.value))}
                    className="font-mono"
                  />
                </Field>
                <Field>
                  <FieldLabel htmlFor="deploy-mem">Memory (MB)</FieldLabel>
                  <Input
                    id="deploy-mem"
                    type="number"
                    min={64}
                    max={65536}
                    value={memoryMb}
                    onChange={(e) => setMemoryMb(Number(e.target.value))}
                    className="font-mono"
                  />
                </Field>
                <Field>
                  <FieldLabel htmlFor="deploy-cpu">CPU (cores)</FieldLabel>
                  <Input
                    id="deploy-cpu"
                    type="number"
                    step="0.25"
                    min={0.1}
                    max={64}
                    value={cpuCores}
                    onChange={(e) => setCpuCores(e.target.value)}
                    placeholder="No limit"
                    className="font-mono"
                  />
                </Field>
                {sourceMode !== 'image' && (
                  <Field>
                    <FieldLabel htmlFor="deploy-dockerfile">
                      Dockerfile <span className="opacity-70">(auto)</span>
                    </FieldLabel>
                    <Input
                      id="deploy-dockerfile"
                      type="text"
                      value={dockerfilePath}
                      onChange={(e) => setDockerfilePath(e.target.value)}
                      placeholder="Dockerfile"
                      className="font-mono"
                    />
                  </Field>
                )}
              </div>

              <Field>
                <FieldLabel htmlFor="deploy-vol">
                  Persistent Volumes <span className="font-normal text-muted-foreground">(optional)</span>
                </FieldLabel>
                <div className="flex items-center gap-2">
                  <Input
                    id="deploy-vol"
                    type="text"
                    value={volumeInput}
                    onChange={(e) => setVolumeInput(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter') {
                        e.preventDefault()
                        const clean = volumeInput.trim()
                        if (clean.startsWith('/') && !volumes.includes(clean)) {
                          setVolumes((prev) => [...prev, clean])
                          setVolumeInput('')
                        }
                      }
                    }}
                    placeholder="/data or /app/uploads"
                    className="flex-1 font-mono"
                  />
                  <Button
                    type="button"
                    variant="secondary"
                    size="compact"
                    leadingIcon={Plus}
                    disabled={!volumeInput.trim().startsWith('/')}
                    onClick={() => {
                      const clean = volumeInput.trim()
                      if (clean.startsWith('/') && !volumes.includes(clean)) {
                        setVolumes((prev) => [...prev, clean])
                        setVolumeInput('')
                      }
                    }}
                  >
                    Add
                  </Button>
                </div>
                {volumes.length > 0 && (
                  <div className="mt-2 flex flex-wrap gap-1.5">
                    {volumes.map((v) => (
                      <span
                        key={v}
                        className="inline-flex items-center gap-1.5 rounded-md border border-border bg-surface px-2 py-0.5 font-mono text-[12px] text-foreground"
                      >
                        <HardDrive className="size-3 text-primary" />
                        {v}
                        <button
                          type="button"
                          onClick={() => setVolumes((prev) => prev.filter((x) => x !== v))}
                          aria-label={`Remove volume ${v}`}
                          className="text-muted-foreground hover:text-fail"
                        >
                          <X className="size-3" />
                        </button>
                      </span>
                    ))}
                  </div>
                )}
              </Field>
            </>
          )}
        </form>

        <DialogFooter className="mt-0 shrink-0 border-t border-border/80 bg-surface/50 px-6 py-3.5">
          <Button
            type="button"
            variant="ghost"
            onClick={() => {
              void desktopApi.cancelGithubConnect()
              onClose()
            }}
          >
            Cancel
          </Button>
          {!githubMissing && (
            <Button
              type="submit"
              form="deploy-app-form"
              variant="primary"
              leadingIcon={Box}
              loading={busy}
              disabled={sourceMode === 'github' && !selectedRepo}
            >
              Create & Deploy
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
