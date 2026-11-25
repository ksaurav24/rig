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
import { Panel } from '@rig/ui/app/page'
import { Badge } from '@rig/ui/badge'
import { Banner, BannerDescription } from '@rig/ui/banner'
import { Button } from '@rig/ui/button'
import { Field, FieldLabel } from '@rig/ui/field'
import { Input } from '@rig/ui/input'
import { desktopApi } from '@/lib/ipc'
import type { DiscoveredRepo, LocalGitStatus } from '@/lib/types'
import { cn } from '@rig/ui/lib/utils'

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
    <div className="space-y-5">
      <Panel
        title="Deploy from Local Git Checkout"
        description={
          <>
            Pick a discovered repository or browse any folder. Rig Desktop deduplicates files with
            zstd blobs and deploys straight to{' '}
            <span className="font-medium text-foreground">{appName}</span>.
          </>
        }
        actions={
          <Button type="button" variant="secondary" size="compact" leadingIcon={FolderGit2} onClick={pickFolder}>
            Choose Folder…
          </Button>
        }
      >
        <div className="space-y-4">
          <div className="flex gap-2.5">
            <Input
              type="text"
              value={repoPath}
              onChange={(e) => setRepoPath(e.target.value)}
              placeholder="/path/to/local/repo"
              className="flex-1 font-mono"
            />
            <Button
              type="button"
              variant="tertiary"
              leadingIcon={RefreshCw}
              loading={busy}
              disabled={!repoPath.trim()}
              onClick={() => void inspect(repoPath)}
            >
              Inspect
            </Button>
          </div>

          <div>
            <div className="mb-2 flex items-center justify-between gap-2">
              <span className="text-[12.5px] font-medium text-muted-foreground">
                Discovered local repositories ({filteredRepos.length})
              </span>
              <div className="relative">
                <Search className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground" />
                <Input
                  size="compact"
                  type="text"
                  value={repoSearch}
                  onChange={(e) => setRepoSearch(e.target.value)}
                  placeholder="Filter repos…"
                  className="w-48 pl-8"
                />
              </div>
            </div>
            <div className="max-h-48 divide-y divide-border/80 overflow-y-auto rounded-lg border border-border bg-surface/60">
              {filteredRepos.length === 0 ? (
                <div className="px-4 py-6 text-center text-[13px] text-muted-foreground">
                  {scanning
                    ? 'Scanning local directories…'
                    : 'No matching repos — use Choose Folder above.'}
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
                    className={cn(
                      'flex w-full items-center justify-between gap-2 px-4 py-2.5 text-left text-[13.5px] transition-colors',
                      repoPath === r.path ? 'bg-active text-foreground' : 'hover:bg-hover',
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
                      <div className="truncate font-mono text-[12px] text-muted-foreground">
                        {r.path}
                      </div>
                    </div>
                    <div className="flex shrink-0 gap-1">
                      {r.stacks.slice(0, 2).map((s) => (
                        <Badge key={s} variant="solid" color="emerald" size="compact">
                          {s}
                        </Badge>
                      ))}
                    </div>
                  </button>
                ))
              )}
            </div>
          </div>
        </div>
      </Panel>

      {status && (
        <Panel>
          <div className="space-y-4 text-[13.5px]">
            <div className="flex flex-wrap items-center gap-4">
              <div className="flex items-center gap-1.5 font-mono text-foreground">
                <GitBranch className="size-4 text-primary" />
                <span>
                  {status.branch ?? (status.is_git_repo ? 'detached HEAD' : 'plain folder')}
                </span>
              </div>
              {status.head_sha && (
                <div className="flex items-center gap-1.5 font-mono text-muted-foreground">
                  <GitCommitHorizontal className="size-4" />
                  <span className="text-foreground">{status.head_sha}</span>
                  {status.head_subject && (
                    <span className="max-w-md truncate font-sans">{status.head_subject}</span>
                  )}
                </div>
              )}
              {status.stacks.length > 0 && (
                <div className="flex gap-1.5">
                  {status.stacks.map((s) => (
                    <Badge key={s} variant="solid" color="emerald" size="compact" className="font-mono">
                      {s}
                    </Badge>
                  ))}
                </div>
              )}
            </div>

            {status.remote_url && (
              <div className="font-mono text-[12.5px] text-muted-foreground">
                origin: {status.remote_url}
              </div>
            )}

            {(status.dirty_files > 0 || status.unpushed_commits > 0) && (
              <Banner status="warning" contrast="high" size="compact" icon={AlertTriangle}>
                <BannerDescription>
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
                </BannerDescription>
              </Banner>
            )}

            <div className="flex flex-wrap items-end justify-between gap-4 border-t border-border/80 pt-4">
              <Field className="w-72">
                <FieldLabel htmlFor="local-git-df">
                  Dockerfile path <span className="opacity-70">(empty = Railpack)</span>
                </FieldLabel>
                <Input
                  id="local-git-df"
                  type="text"
                  value={dockerfile}
                  onChange={(e) => setDockerfile(e.target.value)}
                  placeholder="Dockerfile"
                  className="font-mono"
                />
              </Field>

              <Button
                type="button"
                variant="primary"
                leadingIcon={Rocket}
                loading={busy}
                onClick={deployNow}
              >
                Deploy this checkout
              </Button>
            </div>
          </div>
        </Panel>
      )}
    </div>
  )
}
