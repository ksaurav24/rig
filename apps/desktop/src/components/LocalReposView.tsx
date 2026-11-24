import { open as openDialog } from '@tauri-apps/plugin-dialog'
import {
  AlertTriangle,
  FolderGit2,
  FolderPlus,
  GitBranch,
  GitCommitHorizontal,
  RefreshCw,
  Rocket,
  Search,
} from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { toast } from 'sonner'
import { desktopApi } from '@/lib/ipc'
import type { DiscoveredRepo } from '@/lib/types'

interface LocalReposViewProps {
  serverName: string
  onDeployRepo: (path: string, name: string) => void
}

export function LocalReposView({ serverName, onDeployRepo }: LocalReposViewProps) {
  const [repos, setRepos] = useState<DiscoveredRepo[]>([])
  const [extraRoots, setExtraRoots] = useState<string[]>([])
  const [search, setSearch] = useState('')
  const [scanning, setScanning] = useState(false)

  async function scan(roots?: string[]) {
    setScanning(true)
    try {
      const found = await desktopApi.discoverLocalRepos(roots && roots.length > 0 ? roots : undefined)
      setRepos(found)
    } catch (e) {
      toast.error(String(e))
    } finally {
      setScanning(false)
    }
  }

  useEffect(() => {
    void scan()
  }, [])

  async function addCustomRoot() {
    const selected = await openDialog({
      directory: true,
      multiple: false,
      title: 'Select a directory containing local repositories or a project folder',
    })
    if (typeof selected === 'string') {
      // Check if the chosen folder is itself a repo/project, or a parent containing repos.
      try {
        const info = await desktopApi.inspectLocalGit(selected)
        if (info.is_git_repo || info.stacks.length > 0) {
          const name = selected.split('/').filter(Boolean).pop() || 'project'
          onDeployRepo(selected, name)
          return
        }
      } catch {
        // fall through to scan as root
      }
      const next = Array.from(new Set([...extraRoots, selected]))
      setExtraRoots(next)
      await scan(next)
    }
  }

  async function browseAndDeployDirect() {
    const selected = await openDialog({
      directory: true,
      multiple: false,
      title: `Pick any local project folder to deploy to ${serverName}`,
    })
    if (typeof selected === 'string') {
      const name = selected.split('/').filter(Boolean).pop() || 'my-app'
      onDeployRepo(selected, name)
    }
  }

  const filtered = useMemo(() => {
    const q = search.trim().toLowerCase()
    if (!q) return repos
    return repos.filter(
      (r) =>
        r.name.toLowerCase().includes(q) ||
        r.path.toLowerCase().includes(q) ||
        r.stacks.some((s) => s.toLowerCase().includes(q)) ||
        (r.branch && r.branch.toLowerCase().includes(q)),
    )
  }, [repos, search])

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-lg font-semibold tracking-tight text-fg">Local Repositories</h1>
          <p className="text-xs text-muted">
            Auto-discovered projects on this laptop — deploy straight to{' '}
            <span className="font-medium text-fg">{serverName}</span> without pushing to a registry.
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <button
            type="button"
            onClick={browseAndDeployDirect}
            className="inline-flex items-center gap-1.5 rounded-lg bg-emerald-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-emerald-500"
          >
            <FolderGit2 className="size-3.5" />
            Browse Any Folder…
          </button>
          <button
            type="button"
            onClick={addCustomRoot}
            className="inline-flex items-center gap-1.5 rounded-lg border border-line bg-surface px-3 py-1.5 text-xs text-fg hover:bg-surface-2"
          >
            <FolderPlus className="size-3.5" />
            Scan Custom Root
          </button>
          <button
            type="button"
            disabled={scanning}
            onClick={() => void scan(extraRoots)}
            className="inline-flex items-center gap-1.5 rounded-lg border border-line bg-surface px-3 py-1.5 text-xs text-fg hover:bg-surface-2 disabled:opacity-50"
          >
            <RefreshCw className={`size-3.5 ${scanning ? 'animate-spin' : ''}`} />
            Rescan
          </button>
        </div>
      </div>

      <div className="relative">
        <Search className="pointer-events-none absolute top-1/2 left-3 size-3.5 -translate-y-1/2 text-muted" />
        <input
          type="text"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder="Search local repos by name, path, branch, or stack (Rust, Node, Docker, Go, Python)…"
          className="w-full rounded-xl border border-line bg-surface py-2 pr-3 pl-9 text-xs text-fg outline-none focus:border-emerald-500"
        />
      </div>

      {filtered.length === 0 ? (
        <div className="rounded-xl border border-line bg-surface p-10 text-center text-xs text-muted">
          {scanning
            ? 'Scanning ~/code, ~/projects, ~/src, ~/dev…'
            : 'No local repositories found in default directories. Click "Browse Any Folder…" or "Scan Custom Root" above.'}
        </div>
      ) : (
        <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
          {filtered.map((repo) => (
            <div
              key={repo.path}
              className="flex flex-col justify-between gap-3 rounded-xl border border-line bg-surface p-4 transition-colors hover:border-emerald-500/40"
            >
              <div className="space-y-1.5">
                <div className="flex items-start justify-between gap-2">
                  <div className="min-w-0">
                    <div className="truncate text-sm font-semibold text-fg">{repo.name}</div>
                    <div className="truncate font-mono text-[11px] text-muted">{repo.path}</div>
                  </div>
                  <div className="flex shrink-0 flex-wrap justify-end gap-1">
                    {repo.stacks.map((s) => (
                      <span
                        key={s}
                        className="rounded bg-surface-2 px-1.5 py-0.5 font-mono text-[10px] text-emerald-300"
                      >
                        {s}
                      </span>
                    ))}
                  </div>
                </div>

                <div className="flex flex-wrap items-center gap-3 pt-1 font-mono text-[11px] text-muted">
                  {repo.branch && (
                    <span className="inline-flex items-center gap-1 text-fg/90">
                      <GitBranch className="size-3 text-emerald-400" />
                      {repo.branch}
                    </span>
                  )}
                  {repo.head_sha && (
                    <span className="inline-flex items-center gap-1">
                      <GitCommitHorizontal className="size-3" />
                      {repo.head_sha}
                    </span>
                  )}
                  {(repo.dirty_files > 0 || repo.unpushed_commits > 0) && (
                    <span className="inline-flex items-center gap-1 text-amber-300">
                      <AlertTriangle className="size-3" />
                      {repo.dirty_files > 0 ? `${repo.dirty_files} dirty` : ''}
                      {repo.dirty_files > 0 && repo.unpushed_commits > 0 ? ' · ' : ''}
                      {repo.unpushed_commits > 0 ? `${repo.unpushed_commits} unpushed` : ''}
                    </span>
                  )}
                </div>

                {repo.head_subject && (
                  <p className="truncate text-[11px] text-muted">{repo.head_subject}</p>
                )}
              </div>

              <div className="flex items-center justify-end border-t border-line pt-2.5">
                <button
                  type="button"
                  onClick={() => onDeployRepo(repo.path, repo.name)}
                  className="inline-flex items-center gap-1.5 rounded-lg bg-emerald-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-emerald-500"
                >
                  <Rocket className="size-3.5" />
                  Deploy to {serverName}
                </button>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
