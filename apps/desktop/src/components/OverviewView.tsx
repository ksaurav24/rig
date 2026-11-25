import {
  Activity,
  Box,
  ChevronRight,
  Database,
  FolderGit2,
  Plus,
  Radio,
  RefreshCw,
  Rocket,
} from 'lucide-react'
import type { ReactNode } from 'react'
import { Page, Panel } from '@rig/ui/app/page'
import { EmptyState, Status, statusTone, type Tone } from '@rig/ui/app/state'
import { Badge } from '@rig/ui/badge'
import { Banner, BannerDescription } from '@rig/ui/banner'
import { Button } from '@rig/ui/button'
import { formatBytes, formatPercent, relativeTime } from '@/lib/format'
import type { ServerSnapshot, ServerView } from '@/lib/types'
import { cn } from '@rig/ui/lib/utils'

interface OverviewViewProps {
  server: ServerView
  snapshot: ServerSnapshot | undefined
  loading: boolean
  onRefresh: () => void
  onOpenApp: (appId: string) => void
  onDeployApp: () => void
  onOpenLocalRepos: () => void
  onOpenDatabases: () => void
  onCreateDatabase: () => void
}

const APP_ORDER: Record<string, number> = {
  failed: 0,
  crashed: 0,
  deploying: 1,
  building: 1,
  running: 2,
  stopped: 3,
}

export function OverviewView({
  server,
  snapshot,
  loading,
  onRefresh,
  onOpenApp,
  onDeployApp,
  onOpenLocalRepos,
  onOpenDatabases,
  onCreateDatabase,
}: OverviewViewProps) {
  const sys = snapshot?.system
  const memPct = sys ? formatPercent(sys.memory_used, sys.memory_total) : 0
  const diskPct = sys ? formatPercent(sys.disk_used, sys.disk_total) : 0
  const cpuPct = sys ? Math.round(sys.cpu_percent) : 0

  const apps = snapshot?.apps ?? []
  const sortedApps = [...apps].sort(
    (a, b) => (APP_ORDER[a.status] ?? 4) - (APP_ORDER[b.status] ?? 4) || a.name.localeCompare(b.name),
  )
  const appsRunning = apps.filter((a) => a.status === 'running').length
  const dbs = snapshot?.databases ?? []
  const dbsRunning = dbs.filter((d) => d.status === 'running').length

  return (
    <Page
      title={server.name}
      description={
        <span className="inline-flex flex-wrap items-center gap-2">
          <span className="font-mono text-[13px]">{server.url}</span>
          {snapshot?.meta?.access && (
            <Badge variant="dot" color="gray" size="compact" className="font-mono uppercase">
              {snapshot.meta.access}
            </Badge>
          )}
          {snapshot?.meta?.server_version && (
            <span className="font-mono text-[12px] text-muted-foreground tabular-nums">
              v{snapshot.meta.server_version}
            </span>
          )}
        </span>
      }
      actions={
        <>
          <Button type="button" variant="primary" leadingIcon={Rocket} onClick={onDeployApp}>
            Deploy App
          </Button>
          <Button type="button" variant="tertiary" leadingIcon={FolderGit2} onClick={onOpenLocalRepos}>
            Local Repos
          </Button>
          <Button type="button" variant="tertiary" leadingIcon={Database} onClick={onCreateDatabase}>
            New Database
          </Button>
          <Button
            type="button"
            variant="tertiary"
            size="icon"
            loading={loading}
            onClick={onRefresh}
            aria-label="Refresh server snapshot"
            title="Refresh snapshot"
          >
            <RefreshCw className="size-4" />
          </Button>
        </>
      }
    >
      {snapshot?.from_cache && (
        <Banner status="warning" contrast="high">
          <BannerDescription>
            Showing cached read-only snapshot while connecting to {server.name}.
          </BannerDescription>
        </Banner>
      )}

      {sys && (!sys.engine || !sys.proxy_ready) && (
        <Banner role="alert" status="error" contrast="high">
          <BannerDescription>
            {!sys.engine
              ? 'Rig cannot reach the container engine. Check that Docker or Podman is running on this host.'
              : 'Rig cannot reach Caddy, so domains are not being routed. Apps keep running until Caddy recovers.'}
          </BannerDescription>
        </Banner>
      )}

      {/* Unified Host Telemetry Strip (matches Rig web dashboard) */}
      <dl className="grid overflow-hidden rounded-xl bg-card shadow-[var(--shadow-2)] sm:grid-cols-2 lg:grid-cols-4 [&>div]:border-b [&>div]:border-border/80 [&>div]:sm:border-r lg:[&>div]:border-b-0 lg:[&>div:last-child]:border-r-0">
        <Gauge
          label="CPU"
          ready={Boolean(sys)}
          used={cpuPct}
          detail="all cores"
        />
        <Gauge
          label="Memory"
          ready={Boolean(sys)}
          used={memPct}
          detail={sys ? `${formatBytes(sys.memory_used)} / ${formatBytes(sys.memory_total)}` : ''}
        />
        <Gauge
          label="Disk"
          ready={Boolean(sys)}
          used={diskPct}
          detail={
            sys && sys.disk_total > 0
              ? `${formatBytes(sys.disk_total - sys.disk_used)} free`
              : 'unavailable'
          }
        />
        <div className="flex flex-col justify-center gap-2.5 px-5 py-4 text-[13.5px]">
          <Health tone={sys?.engine ? 'ok' : 'fail'} label="Engine">
            {sys?.engine ? `${sys.engine} ${sys.engine_version ?? ''}`.trim() : 'Unreachable'}
          </Health>
          <Health tone={sys?.proxy_ready ? 'ok' : 'fail'} label="Proxy">
            {sys?.proxy_ready ? 'Caddy serving' : 'Offline'}
          </Health>
          <Health tone={appsRunning > 0 ? 'ok' : 'idle'} label="Workloads">
            {sys ? `${sys.apps_running}/${sys.apps_total} apps · ${dbs.length} DBs` : '—'}
          </Health>
        </div>
      </dl>

      {/* Apps + Live Event Feed */}
      <div className="grid grid-cols-1 gap-5 lg:grid-cols-3">
        <Panel
          title={
            <>
              <Activity className="size-4 text-primary" />
              Applications
            </>
          }
          description={`${appsRunning}/${apps.length} running · ${snapshot?.projects.length ?? 0} projects`}
          actions={
            <Button
              type="button"
              variant="secondary"
              size="compact"
              leadingIcon={Plus}
              onClick={onDeployApp}
            >
              Deploy
            </Button>
          }
          flush={sortedApps.length > 0}
          className="lg:col-span-2"
        >
          {sortedApps.length === 0 ? (
            <EmptyState
              icon={Box}
              title="No applications deployed yet"
              action={
                <>
                  <Button type="button" variant="primary" leadingIcon={Rocket} onClick={onDeployApp}>
                    Deploy App
                  </Button>
                  <Button
                    type="button"
                    variant="tertiary"
                    leadingIcon={FolderGit2}
                    onClick={onOpenLocalRepos}
                  >
                    Browse Local Repos
                  </Button>
                </>
              }
            >
              Deploy a local repository from this machine, a Docker image, or a Git URL.
            </EmptyState>
          ) : (
            <ul className="divide-y divide-border/80">
              {sortedApps.map((app) => (
                <li key={app.id}>
                  <button
                    type="button"
                    onClick={() => onOpenApp(app.id)}
                    className="group flex w-full items-center gap-4 px-5 py-3 text-left text-[14px] transition-colors duration-80 outline-none hover:bg-hover focus-visible:bg-hover"
                  >
                    <div className="min-w-0 flex-1">
                      <div className="truncate">
                        <span className="font-medium text-foreground">{app.name}</span>
                        <span className="text-muted-foreground"> · {app.project_name}</span>
                      </div>
                      <div className="mt-0.5 truncate font-mono text-[12px] text-muted-foreground">
                        {app.source_kind} · port {app.port}
                        {app.git_branch ? ` · ${app.git_branch}` : ''}
                        {app.domains[0] ? ` · ${app.domains[0]}` : ''}
                      </div>
                    </div>
                    <Status tone={statusTone(app.status)} className="shrink-0">
                      {app.status}
                    </Status>
                    <ChevronRight
                      className="size-4 shrink-0 text-muted-foreground/60 transition-transform duration-100 group-hover:translate-x-0.5 group-hover:text-foreground"
                      aria-hidden="true"
                    />
                  </button>
                </li>
              ))}
            </ul>
          )}
        </Panel>

        <Panel
          title={
            <>
              <Radio className="size-4 text-primary" />
              Live Event Feed
            </>
          }
          description="Real-time SSE stream"
          actions={
            <Badge variant="dot" color="emerald" size="compact" className="font-mono">
              /events
            </Badge>
          }
          flush
        >
          <div className="max-h-96 divide-y divide-border/80 overflow-y-auto">
            {(snapshot?.recent_events ?? []).length === 0 ? (
              <div className="px-5 py-10 text-center text-[13.5px] leading-relaxed text-muted-foreground">
                Connected to live event stream. Deploys, crashes, and alerts appear here in real
                time.
              </div>
            ) : (
              [...(snapshot?.recent_events ?? [])].reverse().map((ev) => (
                <div key={ev.id} className="px-5 py-3 text-[13.5px]">
                  <div className="flex items-center justify-between gap-2">
                    <span className="font-medium text-foreground">{ev.title}</span>
                    <span className="shrink-0 font-mono text-[11.5px] text-muted-foreground tabular-nums">
                      {relativeTime(ev.at)}
                    </span>
                  </div>
                  {ev.detail && (
                    <p className="mt-1 line-clamp-2 text-[12.5px] text-muted-foreground">
                      {ev.detail}
                    </p>
                  )}
                  <Badge variant="solid" color="gray" size="compact" className="mt-2 font-mono">
                    {ev.kind}
                  </Badge>
                </div>
              ))
            )}
          </div>
        </Panel>
      </div>

      {/* Databases quick summary */}
      {dbs.length > 0 && (
        <Panel
          title={
            <>
              <Database className="size-4 text-busy" />
              Managed Databases
            </>
          }
          description={`${dbsRunning}/${dbs.length} running`}
          actions={
            <Button
              type="button"
              variant="secondary"
              size="compact"
              leadingIcon={Plus}
              onClick={onCreateDatabase}
            >
              New Database
            </Button>
          }
        >
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
            {dbs.map((db) => (
              <button
                key={db.id}
                type="button"
                onClick={onOpenDatabases}
                className="group flex items-center justify-between gap-3 rounded-lg border border-border/80 bg-surface/60 px-4 py-3 text-left transition-colors duration-80 hover:bg-hover"
              >
                <div className="min-w-0">
                  <div className="truncate text-[14px] font-medium text-foreground">{db.name}</div>
                  <div className="mt-0.5 truncate font-mono text-[12px] text-muted-foreground">
                    {db.engine} {db.version} · :{db.port}
                  </div>
                </div>
                <div className="flex shrink-0 items-center gap-2">
                  <Status tone={statusTone(db.status)} size="compact">
                    {db.status}
                  </Status>
                  <ChevronRight className="size-3.5 text-muted-foreground/60 transition-transform group-hover:translate-x-0.5 group-hover:text-foreground" />
                </div>
              </button>
            ))}
          </div>
        </Panel>
      )}
    </Page>
  )
}

function Gauge({
  label,
  used,
  detail,
  ready,
}: {
  label: string
  used: number
  detail: string
  ready: boolean
}) {
  const tone = used >= 90 ? 'bg-fail' : used >= 75 ? 'bg-warn' : 'bg-primary'
  return (
    <div className="flex flex-col gap-2 px-5 py-4">
      <dt className="text-[13px] font-medium text-muted-foreground">{label}</dt>
      <dd className="flex flex-col gap-2.5">
        <span className="flex items-baseline gap-2.5">
          <span className="font-mono text-[26px] leading-none font-semibold tracking-tight text-foreground tabular-nums">
            {ready ? `${used}%` : '—'}
          </span>
          <span className="truncate text-[12.5px] text-muted-foreground">{ready ? detail : 'Loading…'}</span>
        </span>
        <span
          role="meter"
          aria-label={`${label} in use`}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={used}
          className="h-1.5 overflow-hidden rounded-full bg-muted"
        >
          <span
            className={cn('block h-full rounded-full transition-[width] duration-500', tone)}
            style={{ width: `${ready ? Math.min(100, Math.max(used, 2)) : 0}%` }}
          />
        </span>
      </dd>
    </div>
  )
}

function Health({ tone, label, children }: { tone: Tone; label: string; children: ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-3">
      <Status tone={tone} size="compact" className="text-muted-foreground">
        {label}
      </Status>
      <span className="min-w-0 truncate text-[13px] font-medium text-foreground">{children}</span>
    </div>
  )
}
