import { open as openDialog } from '@tauri-apps/plugin-dialog'
import {
  AlertTriangle,
  Box,
  FolderGit2,
  GitBranch,
  Package,
  RefreshCw,
  Rocket,
  Search,
  X,
} from 'lucide-react'
import { useEffect, useMemo, useState, type FormEvent } from 'react'
import { toast } from 'sonner'
import { desktopApi } from '@/lib/ipc'
import type { App, DiscoveredRepo, Project, SourceKind } from '@/lib/types'

interface DeployModalProps {
  open: boolean
  serverId: string
  projects: Project[]
  initialLocalPath?: string | null
  initialName?: string | null
  onClose: () => void
  onDeployed: (app: App) => void
}

export function DeployModal({
  open,
  serverId,
  projects,
  initialLocalPath,
  initialName,
  onClose,
  onDeployed,
}: DeployModalProps) {
  const [sourceKind, setSourceKind] = useState<SourceKind>(initialLocalPath ? 'local' : 'local')
  const [projectId, setProjectId] = useState<string>(projects[0]?.id ?? '__new__')
  const [newProjectName, setNewProjectName] = useState('Default')
  const [appName, setAppName] = useState('')
  const [port, setPort] = useState(3000)
  const [memoryMb, setMemoryMb] = useState(512)
  const [dockerfilePath, setDockerfilePath] = useState('Dockerfile')

  // Image mode
  const [image, setImage] = useState('nginx:alpine')

  // Git mode
  const [gitUrl, setGitUrl] = useState('')
  const [gitBranch, setGitBranch] = useState('main')

  // Local mode
  const [localPath, setLocalPath] = useState('')
  const [repos, setRepos] = useState<DiscoveredRepo[]>([])
  const [repoSearch, setRepoSearch] = useState('')
  const [scanning, setScanning] = useState(false)

  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (!open) return
    setError(null)
    setProjectId(projects[0]?.id ?? '__new__')
    if (initialLocalPath) {
      setSourceKind('local')
      setLocalPath(initialLocalPath)
      const folderName =
        initialName || initialLocalPath.split('/').filter(Boolean).pop() || 'my-app'
      setAppName(folderName)
    } else {
      setAppName('')
    }
    setScanning(true)
    void desktopApi
      .discoverLocalRepos()
      .then(setRepos)
      .catch(() => {})
      .finally(() => setScanning(false))
  }, [initialLocalPath, initialName, open, projects])

  const filteredRepos = useMemo(() => {
    const q = repoSearch.trim().toLowerCase()
    if (!q) return repos
    return repos.filter(
      (r) =>
        r.name.toLowerCase().includes(q) ||
        r.path.toLowerCase().includes(q) ||
        r.stacks.some((s) => s.toLowerCase().includes(q)),
    )
  }, [repoSearch, repos])

  if (!open) return null

  async function pickFolder() {
    const selected = await openDialog({
      directory: true,
      multiple: false,
      title: 'Select local project folder to deploy',
    })
    if (typeof selected === 'string') {
      setLocalPath(selected)
      const folderName = selected.split('/').filter(Boolean).pop() || 'my-app'
      if (!appName.trim()) setAppName(folderName)
      try {
        const info = await desktopApi.inspectLocalGit(selected)
        if (!info.has_dockerfile) {
          setDockerfilePath('')
        } else {
          setDockerfilePath('Dockerfile')
        }
      } catch {
        // ignore
      }
    }
  }

  async function handleSubmit(e: FormEvent) {
    e.preventDefault()
    setError(null)
    if (!appName.trim()) {
      setError('Enter an app name')
      return
    }
    if (sourceKind === 'local' && !localPath.trim()) {
      setError('Choose or enter a local folder to deploy')
      return
    }
    if (sourceKind === 'image' && !image.trim()) {
      setError('Enter a Docker image (e.g. nginx:alpine)')
      return
    }
    if (sourceKind === 'git' && !gitUrl.trim()) {
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

      const created = await desktopApi.createApp(serverId, {
        project_id: targetProjectId,
        name: appName.trim(),
        source_kind: sourceKind,
        image: sourceKind === 'image' ? image.trim() : null,
        git_url: sourceKind === 'git' ? gitUrl.trim() : null,
        git_branch: sourceKind === 'git' ? gitBranch.trim() || 'main' : null,
        dockerfile_path:
          sourceKind === 'image' ? null : dockerfilePath.trim() || 'Dockerfile',
        port: Number(port) || 3000,
        memory_mb: Number(memoryMb) || 512,
      })

      if (sourceKind === 'local') {
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
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4">
      <div className="flex max-h-[90vh] w-full max-w-2xl flex-col overflow-hidden rounded-2xl border border-line bg-surface shadow-2xl">
        <div className="flex items-center justify-between border-b border-line px-5 py-4">
          <div className="flex items-center gap-2.5">
            <div className="flex size-8 items-center justify-center rounded-lg bg-emerald-500/15 text-emerald-400">
              <Rocket className="size-4" />
            </div>
            <div>
              <h2 className="text-sm font-semibold text-fg">Deploy New Application</h2>
              <p className="text-xs text-muted">
                Deploy from a local repository, a Docker image, or a Git URL
              </p>
            </div>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="rounded-lg p-1.5 text-muted hover:bg-surface-2 hover:text-fg"
          >
            <X className="size-4" />
          </button>
        </div>

        <form onSubmit={handleSubmit} className="min-h-0 flex-1 space-y-4 overflow-y-auto p-5">
          {error && (
            <div className="flex items-start gap-2 rounded-lg border border-rose-500/30 bg-rose-500/10 px-3 py-2 text-xs text-rose-300">
              <AlertTriangle className="mt-0.5 size-3.5 shrink-0" />
              <span>{error}</span>
            </div>
          )}

          {/* Source Kind Switcher */}
          <div className="grid grid-cols-3 gap-1.5 rounded-xl bg-bg p-1 text-xs">
            {(
              [
                { id: 'local', label: 'Local Folder / Repo', icon: <FolderGit2 className="size-3.5" /> },
                { id: 'image', label: 'Docker Image', icon: <Package className="size-3.5" /> },
                { id: 'git', label: 'Git Repository', icon: <GitBranch className="size-3.5" /> },
              ] as const
            ).map((t) => (
              <button
                key={t.id}
                type="button"
                onClick={() => setSourceKind(t.id)}
                className={`flex items-center justify-center gap-1.5 rounded-lg py-2 font-medium transition-colors ${
                  sourceKind === t.id ? 'bg-surface-2 text-fg ring-1 ring-line' : 'text-muted hover:text-fg'
                }`}
              >
                {t.icon}
                {t.label}
              </button>
            ))}
          </div>

          {/* Local Folder Picker + Discovered Repos */}
          {sourceKind === 'local' && (
            <div className="space-y-2.5 rounded-xl border border-line bg-bg/60 p-3.5">
              <div className="flex items-center justify-between gap-2">
                <label className="text-xs font-medium text-fg">Local project directory</label>
                <button
                  type="button"
                  onClick={pickFolder}
                  className="inline-flex items-center gap-1.5 rounded-lg border border-emerald-500/40 bg-emerald-500/10 px-2.5 py-1 text-[11px] font-medium text-emerald-300 hover:bg-emerald-500/20"
                >
                  <FolderGit2 className="size-3.5" />
                  Browse Folder…
                </button>
              </div>
              <input
                type="text"
                value={localPath}
                onChange={(e) => setLocalPath(e.target.value)}
                placeholder="/home/you/code/my-app"
                className="w-full rounded-lg border border-line bg-bg px-3 py-1.5 font-mono text-xs text-fg outline-none focus:border-emerald-500"
              />

              {/* Auto-discovered local repos list */}
              <div className="pt-1">
                <div className="mb-1.5 flex items-center justify-between gap-2">
                  <span className="text-[11px] font-medium text-muted">
                    Discovered local repos ({filteredRepos.length})
                  </span>
                  <div className="flex items-center gap-1.5">
                    <div className="relative">
                      <Search className="pointer-events-none absolute top-1/2 left-2 size-3 -translate-y-1/2 text-muted" />
                      <input
                        type="text"
                        value={repoSearch}
                        onChange={(e) => setRepoSearch(e.target.value)}
                        placeholder="Filter repos…"
                        className="w-40 rounded-md border border-line bg-bg py-0.5 pr-2 pl-6 text-[11px] text-fg outline-none focus:border-emerald-500"
                      />
                    </div>
                    <button
                      type="button"
                      onClick={() => {
                        setScanning(true)
                        void desktopApi
                          .discoverLocalRepos()
                          .then(setRepos)
                          .finally(() => setScanning(false))
                      }}
                      className="rounded p-1 text-muted hover:bg-surface-2 hover:text-fg"
                      title="Rescan local repos"
                    >
                      <RefreshCw className={`size-3 ${scanning ? 'animate-spin' : ''}`} />
                    </button>
                  </div>
                </div>
                <div className="max-h-36 divide-y divide-line overflow-y-auto rounded-lg border border-line bg-bg">
                  {filteredRepos.length === 0 ? (
                    <div className="px-3 py-4 text-center text-[11px] text-muted">
                      {scanning
                        ? 'Scanning ~/code, ~/projects, ~/src…'
                        : 'No repos matched — use Browse Folder above.'}
                    </div>
                  ) : (
                    filteredRepos.map((r) => {
                      const active = localPath === r.path
                      return (
                        <button
                          key={r.path}
                          type="button"
                          onClick={() => {
                            setLocalPath(r.path)
                            setAppName(r.name)
                            setDockerfilePath(r.has_dockerfile ? 'Dockerfile' : '')
                          }}
                          className={`flex w-full items-center justify-between gap-2 px-3 py-2 text-left text-xs ${
                            active ? 'bg-emerald-500/15 text-fg' : 'hover:bg-surface-2/60'
                          }`}
                        >
                          <div className="min-w-0">
                            <div className="flex items-center gap-1.5 font-medium text-fg">
                              <span className="truncate">{r.name}</span>
                              {r.branch && (
                                <span className="rounded bg-surface-2 px-1 py-0.5 font-mono text-[10px] text-muted">
                                  {r.branch}
                                </span>
                              )}
                            </div>
                            <div className="truncate font-mono text-[10px] text-muted">{r.path}</div>
                          </div>
                          <div className="flex shrink-0 items-center gap-1">
                            {r.stacks.slice(0, 2).map((s) => (
                              <span
                                key={s}
                                className="rounded bg-surface-2 px-1.5 py-0.5 text-[10px] text-emerald-300"
                              >
                                {s}
                              </span>
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

          {sourceKind === 'image' && (
            <div>
              <label className="mb-1 block text-xs font-medium text-fg">Container Image</label>
              <input
                type="text"
                required
                value={image}
                onChange={(e) => setImage(e.target.value)}
                placeholder="nginx:alpine or ghcr.io/org/service:latest"
                className="w-full rounded-lg border border-line bg-bg px-3 py-2 font-mono text-xs text-fg outline-none focus:border-emerald-500"
              />
            </div>
          )}

          {sourceKind === 'git' && (
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
              <div className="sm:col-span-2">
                <label className="mb-1 block text-xs font-medium text-fg">Git Repository URL</label>
                <input
                  type="text"
                  required
                  value={gitUrl}
                  onChange={(e) => setGitUrl(e.target.value)}
                  placeholder="https://github.com/owner/repo.git"
                  className="w-full rounded-lg border border-line bg-bg px-3 py-2 font-mono text-xs text-fg outline-none focus:border-emerald-500"
                />
              </div>
              <div>
                <label className="mb-1 block text-xs font-medium text-fg">Branch</label>
                <input
                  type="text"
                  value={gitBranch}
                  onChange={(e) => setGitBranch(e.target.value)}
                  placeholder="main"
                  className="w-full rounded-lg border border-line bg-bg px-3 py-2 font-mono text-xs text-fg outline-none focus:border-emerald-500"
                />
              </div>
            </div>
          )}

          {/* Core App Metadata */}
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <div>
              <label className="mb-1 block text-xs font-medium text-fg">Project</label>
              <select
                value={projectId}
                onChange={(e) => setProjectId(e.target.value)}
                className="w-full rounded-lg border border-line bg-bg px-3 py-2 text-xs text-fg outline-none focus:border-emerald-500"
              >
                {projects.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.name}
                  </option>
                ))}
                <option value="__new__">+ Create new project…</option>
              </select>
            </div>
            <div>
              <label className="mb-1 block text-xs font-medium text-fg">App Name</label>
              <input
                type="text"
                required
                value={appName}
                onChange={(e) => setAppName(e.target.value)}
                placeholder="api, web, worker…"
                className="w-full rounded-lg border border-line bg-bg px-3 py-2 text-xs text-fg outline-none focus:border-emerald-500"
              />
            </div>
          </div>

          {projectId === '__new__' && (
            <div>
              <label className="mb-1 block text-xs font-medium text-fg">New Project Name</label>
              <input
                type="text"
                required
                value={newProjectName}
                onChange={(e) => setNewProjectName(e.target.value)}
                placeholder="Production, Homelab…"
                className="w-full rounded-lg border border-line bg-bg px-3 py-2 text-xs text-fg outline-none focus:border-emerald-500"
              />
            </div>
          )}

          <div className="grid grid-cols-3 gap-3">
            <div>
              <label className="mb-1 block text-xs font-medium text-fg">Container Port</label>
              <input
                type="number"
                min={1}
                max={65535}
                value={port}
                onChange={(e) => setPort(Number(e.target.value))}
                className="w-full rounded-lg border border-line bg-bg px-3 py-1.5 font-mono text-xs text-fg outline-none focus:border-emerald-500"
              />
            </div>
            <div>
              <label className="mb-1 block text-xs font-medium text-fg">Memory Limit (MB)</label>
              <input
                type="number"
                min={64}
                max={65536}
                value={memoryMb}
                onChange={(e) => setMemoryMb(Number(e.target.value))}
                className="w-full rounded-lg border border-line bg-bg px-3 py-1.5 font-mono text-xs text-fg outline-none focus:border-emerald-500"
              />
            </div>
            {sourceKind !== 'image' && (
              <div>
                <label className="mb-1 block text-xs font-medium text-fg">
                  Dockerfile <span className="text-muted">(empty = Railpack)</span>
                </label>
                <input
                  type="text"
                  value={dockerfilePath}
                  onChange={(e) => setDockerfilePath(e.target.value)}
                  placeholder="Dockerfile"
                  className="w-full rounded-lg border border-line bg-bg px-3 py-1.5 font-mono text-xs text-fg outline-none focus:border-emerald-500"
                />
              </div>
            )}
          </div>

          <div className="flex items-center justify-end gap-2 border-t border-line pt-4">
            <button
              type="button"
              onClick={onClose}
              className="rounded-lg px-3.5 py-2 text-xs text-muted hover:bg-surface-2 hover:text-fg"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={busy}
              className="inline-flex items-center gap-1.5 rounded-lg bg-emerald-600 px-4 py-2 text-xs font-semibold text-white hover:bg-emerald-500 disabled:opacity-50"
            >
              <Box className="size-3.5" />
              {busy ? 'Deploying…' : 'Create & Deploy'}
            </button>
          </div>
        </form>
      </div>
    </div>
  )
}
