import { open } from '@tauri-apps/plugin-dialog'
import {
  AlertTriangle,
  FolderGit2,
  GitBranch,
  GitCommitHorizontal,
  RefreshCw,
  Rocket,
  Search,
} from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { toast } from 'sonner'
import { desktopApi } from '@/lib/ipc'
import type { DiscoveredRepo, LocalGitStatus } from '@/lib/types'

interface LocalGitPanelProps {
  serverId: string
  appId: string
  appName: string
  defaultDockerfile: string
  onDeployed: () => void
}

export function LocalGitPanel({
  serverId,
  appId,
  appName,
  defaultDockerfile,
  onDeployed,
}: LocalGitPanelProps) {
  const [repoPath, setRepoPath] = useState('')
  const [status, setStatus] = useState<LocalGitStatus | null>(null)
  const [dockerfile, setDockerfile] = useState(defaultDockerfile || 'Dockerfile')
  const [busy, setBusy] = useState(false)
  const [repos, setRepos] = useState<DiscoveredRepo[]>([])
  const [repoSearch, setRepoSearch] = useState('')
  const [scanning, setScanning] = useState(false)

  useEffect(() => {
    setScanning(true)
    void desktopApi
      .discoverLocalRepos()
      .then(setRepos)
      .catch(() => {})
      .finally(() => setScanning(false))
  }, [])

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

  async function inspect(path: string) {
    if (!path.trim()) return
    setBusy(true)
    try {
      const info = await desktopApi.inspectLocalGit(path.trim())
      setStatus(info)
      setDockerfile(info.has_dockerfile ? defaultDockerfile || 'Dockerfile' : '')
    } catch (e) {
      toast.error(String(e))
    } finally {
      setBusy(false)
    }
  }

  async function pickFolder() {
    const selected = await open({
      directory: true,
      multiple: false,
      title: `Choose local Git checkout for ${appName}`,
    })
    if (typeof selected === 'string') {
      setRepoPath(selected)
      await inspect(selected)
    }
  }

  async function deployNow() {
    if (!repoPath.trim()) return
    setBusy(true)
    try {
      const dep = await desktopApi.deployLocalGit(
        serverId,
        appId,
        repoPath.trim(),
        dockerfile.trim() || undefined,
      )
      toast.success(`Started deployment ${dep.id.slice(-6)} from local checkout`)
      onDeployed()
    } catch (e) {
      toast.error(String(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="space-y-4">
      <div className="space-y-3 rounded-xl border border-line bg-surface p-4">
        <div className="flex items-start justify-between gap-3">
          <div>
            <h3 className="text-sm font-semibold text-fg">Deploy from Local Git Checkout</h3>
            <p className="mt-0.5 text-xs text-muted">
              Pick a discovered repository or browse any folder. Rig Desktop deduplicates files with
              zstd blobs and deploys straight to <span className="font-medium text-fg">{appName}</span>.
            </p>
          </div>
          <button
            type="button"
            onClick={pickFolder}
            className="inline-flex shrink-0 items-center gap-1.5 rounded-lg border border-line bg-surface-2 px-3 py-1.5 text-xs font-medium text-fg hover:bg-bg"
          >
            <FolderGit2 className="size-3.5 text-emerald-400" />
            Choose Folder…
          </button>
        </div>

        <div className="flex gap-2">
          <input
            type="text"
            value={repoPath}
            onChange={(e) => setRepoPath(e.target.value)}
            placeholder="/path/to/local/repo"
            className="flex-1 rounded-lg border border-line bg-bg px-3 py-1.5 font-mono text-xs text-fg outline-none focus:border-emerald-500"
          />
          <button
            type="button"
            disabled={busy || !repoPath.trim()}
            onClick={() => void inspect(repoPath)}
            className="inline-flex items-center gap-1.5 rounded-lg border border-line bg-surface-2 px-3 py-1.5 text-xs text-fg hover:bg-bg disabled:opacity-50"
          >
            <RefreshCw className="size-3.5" />
            Inspect
          </button>
        </div>

        {/* Discovered local repositories quick-picker */}
        <div>
          <div className="mb-1.5 flex items-center justify-between gap-2">
            <span className="text-[11px] font-medium text-muted">
              Discovered local repositories ({filteredRepos.length})
            </span>
            <div className="relative">
              <Search className="pointer-events-none absolute top-1/2 left-2 size-3 -translate-y-1/2 text-muted" />
              <input
                type="text"
                value={repoSearch}
                onChange={(e) => setRepoSearch(e.target.value)}
                placeholder="Filter repos…"
                className="w-44 rounded-md border border-line bg-bg py-0.5 pr-2 pl-6 text-[11px] text-fg outline-none focus:border-emerald-500"
              />
            </div>
          </div>
          <div className="max-h-40 divide-y divide-line overflow-y-auto rounded-lg border border-line bg-bg">
            {filteredRepos.length === 0 ? (
              <div className="px-3 py-4 text-center text-[11px] text-muted">
                {scanning ? 'Scanning local directories…' : 'No matching repos — use Choose Folder.'}
              </div>
            ) : (
              filteredRepos.map((r) => (
                <button
                  key={r.path}
                  type="button"
                  onClick={() => {
                    setRepoPath(r.path)
                    void inspect(r.path)
                  }}
                  className={`flex w-full items-center justify-between gap-2 px-3 py-2 text-left text-xs ${
                    repoPath === r.path ? 'bg-emerald-500/15 text-fg' : 'hover:bg-surface-2/60'
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
                  <div className="flex shrink-0 gap-1">
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
              ))
            )}
          </div>
        </div>
      </div>

      {status && (
        <div className="space-y-3 rounded-xl border border-line bg-surface p-4 text-xs">
          <div className="flex flex-wrap items-center gap-4">
            <div className="flex items-center gap-1.5 font-mono text-fg">
              <GitBranch className="size-3.5 text-emerald-400" />
              <span>{status.branch ?? (status.is_git_repo ? 'detached HEAD' : 'plain folder')}</span>
            </div>
            {status.head_sha && (
              <div className="flex items-center gap-1.5 font-mono text-muted">
                <GitCommitHorizontal className="size-3.5" />
                <span className="text-fg">{status.head_sha}</span>
                {status.head_subject && (
                  <span className="max-w-md truncate font-sans">{status.head_subject}</span>
                )}
              </div>
            )}
            {status.stacks.length > 0 && (
              <div className="flex gap-1">
                {status.stacks.map((s) => (
                  <span
                    key={s}
                    className="rounded bg-surface-2 px-1.5 py-0.5 font-mono text-[10px] text-emerald-300"
                  >
                    {s}
                  </span>
                ))}
              </div>
            )}
          </div>

          {status.remote_url && (
            <div className="font-mono text-[11px] text-muted">origin: {status.remote_url}</div>
          )}

          {(status.dirty_files > 0 || status.unpushed_commits > 0) && (
            <div className="flex items-start gap-2 rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-amber-200">
              <AlertTriangle className="mt-0.5 size-4 shrink-0 text-amber-400" />
              <div>
                {status.dirty_files > 0 && (
                  <div>
                    {status.dirty_files} uncommitted file{status.dirty_files === 1 ? '' : 's'} in
                    working tree — your working copy will be uploaded directly.
                  </div>
                )}
                {status.unpushed_commits > 0 && (
                  <div>
                    {status.unpushed_commits} local commit
                    {status.unpushed_commits === 1 ? '' : 's'} not pushed to origin.
                  </div>
                )}
              </div>
            </div>
          )}

          <div className="flex flex-wrap items-end justify-between gap-3 pt-1">
            <div>
              <label className="mb-1 block text-[11px] text-muted">
                Dockerfile path <span className="text-muted">(empty = Railpack auto-build)</span>
              </label>
              <input
                type="text"
                value={dockerfile}
                onChange={(e) => setDockerfile(e.target.value)}
                placeholder="Dockerfile"
                className="w-64 rounded-lg border border-line bg-bg px-2.5 py-1.5 font-mono text-xs text-fg outline-none focus:border-emerald-500"
              />
            </div>

            <button
              type="button"
              disabled={busy}
              onClick={deployNow}
              className="inline-flex items-center gap-1.5 rounded-lg bg-emerald-600 px-4 py-2 text-xs font-medium text-white hover:bg-emerald-500 disabled:opacity-50"
            >
              <Rocket className="size-3.5" />
              {busy ? 'Uploading & deploying…' : 'Deploy this checkout'}
            </button>
          </div>
        </div>
      )}
    </div>
  )
}
