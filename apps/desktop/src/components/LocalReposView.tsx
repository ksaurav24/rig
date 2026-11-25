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
import { Page } from '@rig/ui/app/page'
import { EmptyState } from '@rig/ui/app/state'
import { Badge } from '@rig/ui/badge'
import { Button } from '@rig/ui/button'
import { Card, CardContent, CardFooter, CardGroup, CardHeader, CardTitle } from '@rig/ui/card'
import { Input } from '@rig/ui/input'
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
    <Page
      title="Local Repositories"
      description={
        <>
          Auto-discovered projects on this machine — deploy straight to{' '}
          <span className="font-medium text-foreground">{serverName}</span> without pushing to a
          registry.
        </>
      }
      actions={
        <>
          <Button
            type="button"
            variant="primary"
            leadingIcon={FolderGit2}
            onClick={browseAndDeployDirect}
          >
            Browse Any Folder…
          </Button>
          <Button type="button" variant="tertiary" leadingIcon={FolderPlus} onClick={addCustomRoot}>
            Scan Custom Root
          </Button>
          <Button
            type="button"
            variant="tertiary"
            leadingIcon={RefreshCw}
            loading={scanning}
            onClick={() => void scan(extraRoots)}
          >
            Rescan
          </Button>
        </>
      }
    >
      <div className="relative">
        <Search className="pointer-events-none absolute top-1/2 left-3.5 size-4 -translate-y-1/2 text-muted-foreground" />
        <Input
          type="text"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder="Search local repos by name, path, branch, or stack (Rust, Node, Docker, Go, Python)…"
          className="pl-10"
        />
      </div>

      {filtered.length === 0 ? (
        <EmptyState
          icon={FolderGit2}
          title={scanning ? 'Scanning local directories…' : 'No local repositories found'}
          action={
            !scanning ? (
              <Button
                type="button"
                variant="primary"
                leadingIcon={FolderGit2}
                onClick={browseAndDeployDirect}
              >
                Browse Any Folder…
              </Button>
            ) : undefined
          }
        >
          {scanning
            ? 'Checking ~/code, ~/projects, ~/src, ~/dev…'
            : 'Use "Browse Any Folder…" or "Scan Custom Root" above to pick a project directory.'}
        </EmptyState>
      ) : (
        <CardGroup columns={2} border="outlined" separated className="grid-cols-1 md:grid-cols-2 gap-3.5">
          {filtered.map((repo) => (
            <Card
              key={repo.path}
              onClick={() => onDeployRepo(repo.path, repo.name)}
              label={`Deploy ${repo.name} to ${serverName}`}
              className="bg-card shadow-[var(--shadow-2)]"
            >
              <CardHeader>
                <div className="flex items-start justify-between gap-2">
                  <div className="min-w-0">
                    <CardTitle className="truncate font-semibold">{repo.name}</CardTitle>
                    <div className="mt-0.5 truncate font-mono text-[12px] text-muted-foreground">
                      {repo.path}
                    </div>
                  </div>
                  <div className="flex shrink-0 flex-wrap justify-end gap-1">
                    {repo.stacks.map((s) => (
                      <Badge key={s} variant="solid" color="emerald" size="compact" className="font-mono">
                        {s}
                      </Badge>
                    ))}
                  </div>
                </div>
              </CardHeader>

              <CardContent className="space-y-1.5">
                <div className="flex flex-wrap items-center gap-3 font-mono text-[12px] text-muted-foreground">
                  {repo.branch && (
                    <span className="inline-flex items-center gap-1.5 text-foreground">
                      <GitBranch className="size-3.5 text-primary" />
                      {repo.branch}
                    </span>
                  )}
                  {repo.head_sha && (
                    <span className="inline-flex items-center gap-1">
                      <GitCommitHorizontal className="size-3.5" />
                      {repo.head_sha}
                    </span>
                  )}
                  {(repo.dirty_files > 0 || repo.unpushed_commits > 0) && (
                    <Badge variant="solid" color="amber" size="compact" icon={AlertTriangle}>
                      {repo.dirty_files > 0 ? `${repo.dirty_files} dirty` : ''}
                      {repo.dirty_files > 0 && repo.unpushed_commits > 0 ? ' · ' : ''}
                      {repo.unpushed_commits > 0 ? `${repo.unpushed_commits} unpushed` : ''}
                    </Badge>
                  )}
                </div>
                {repo.head_subject && (
                  <p className="truncate text-[12.5px] text-muted-foreground">{repo.head_subject}</p>
                )}
              </CardContent>

              <CardFooter className="mt-auto justify-end border-t border-border/70 pt-3">
                <Button
                  type="button"
                  variant="primary"
                  size="compact"
                  leadingIcon={Rocket}
                  onClick={() => onDeployRepo(repo.path, repo.name)}
                >
                  Deploy to {serverName}
                </Button>
              </CardFooter>
            </Card>
          ))}
        </CardGroup>
      )}
    </Page>
  )
}
