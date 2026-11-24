import {
  Activity,
  Box,
  Cpu,
  Database,
  FolderGit2,
  HardDrive,
  MemoryStick,
  Plus,
  Radio,
  RefreshCw,
  Rocket,
} from 'lucide-react'
import { formatBytes, formatPercent, relativeTime } from '@/lib/format'
import type { ServerSnapshot, ServerView } from '@/lib/types'

interface OverviewViewProps {
  server: ServerView
  snapshot: ServerSnapshot | undefined
  loading: boolean
  onRefresh: () => void
  onOpenApp: (appId: string) => void
  onDeployApp: () => void
  onOpenLocalRepos: () => void
  onCreateDatabase: () => void
}

export function OverviewView({
  server,
  snapshot,
  loading,
  onRefresh,
  onOpenApp,
  onDeployApp,
  onOpenLocalRepos,
  onCreateDatabase,
}: OverviewViewProps) {
  const sys = snapshot?.system
  const memPct = sys ? formatPercent(sys.memory_used, sys.memory_total) : 0
  const diskPct = sys ? formatPercent(sys.disk_used, sys.disk_total) : 0
  const cpuPct = sys ? Math.round(sys.cpu_percent) : 0

  return (
    <div className="space-y-5">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-lg font-semibold tracking-tight text-fg">{server.name}</h1>
          <div className="mt-0.5 flex flex-wrap items-center gap-2 text-xs text-muted">
            <span className="font-mono">{server.url}</span>
            {snapshot?.meta?.mode && (
              <span className="rounded bg-surface-2 px-1.5 py-0.5 font-mono text-[10px] text-fg uppercase">
                {snapshot.meta.mode}
              </span>
            )}
            {snapshot?.meta?.access && (
              <span className="rounded bg-surface-2 px-1.5 py-0.5 font-mono text-[10px] text-muted uppercase">
                {snapshot.meta.access}
              </span>
            )}
            {snapshot?.from_cache && (
              <span className="rounded bg-amber-500/15 px-1.5 py-0.5 text-[10px] font-medium text-amber-300">
                Offline · Cached read-only view
              </span>
            )}
          </div>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <button
            type="button"
            onClick={onDeployApp}
            className="inline-flex items-center gap-1.5 rounded-lg bg-emerald-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-emerald-500"
          >
            <Rocket className="size-3.5" />
            Deploy App
          </button>
          <button
            type="button"
            onClick={onOpenLocalRepos}
            className="inline-flex items-center gap-1.5 rounded-lg border border-line bg-surface px-3 py-1.5 text-xs text-fg hover:bg-surface-2"
          >
            <FolderGit2 className="size-3.5 text-emerald-400" />
            Local Repos
          </button>
          <button
            type="button"
            onClick={onCreateDatabase}
            className="inline-flex items-center gap-1.5 rounded-lg border border-line bg-surface px-3 py-1.5 text-xs text-fg hover:bg-surface-2"
          >
            <Database className="size-3.5 text-sky-400" />
            New Database
          </button>
          {snapshot && (
            <span className="font-mono text-[11px] text-muted tabular-nums">
              {snapshot.fetch_ms}ms
            </span>
          )}
          <button
            type="button"
            onClick={onRefresh}
            disabled={loading}
            className="inline-flex items-center gap-1.5 rounded-lg border border-line bg-surface px-2.5 py-1.5 text-xs text-fg hover:bg-surface-2 disabled:opacity-50"
          >
            <RefreshCw className={`size-3.5 ${loading ? 'animate-spin' : ''}`} />
          </button>
        </div>
      </div>

      {/* Host Metric Cards */}
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-4">
        <MetricCard
          icon={<Cpu className="size-4 text-emerald-400" />}
          label="CPU"
          value={sys ? `${cpuPct}%` : '—'}
          sub={sys?.engine ? `${sys.engine} ${sys.engine_version ?? ''}` : 'Engine probing…'}
          percent={cpuPct}
        />
        <MetricCard
          icon={<MemoryStick className="size-4 text-sky-400" />}
          label="Memory"
          value={sys ? `${memPct}%` : '—'}
          sub={sys ? `${formatBytes(sys.memory_used)} / ${formatBytes(sys.memory_total)}` : '—'}
          percent={memPct}
        />
        <MetricCard
          icon={<HardDrive className="size-4 text-violet-400" />}
          label="Disk"
          value={sys ? `${diskPct}%` : '—'}
          sub={sys ? `${formatBytes(sys.disk_used)} / ${formatBytes(sys.disk_total)}` : '—'}
          percent={diskPct}
        />
        <MetricCard
          icon={<Box className="size-4 text-amber-400" />}
          label="Workloads"
          value={sys ? `${sys.apps_running} / ${sys.apps_total}` : '—'}
          sub={
            sys
              ? `Proxy ${sys.proxy_ready ? 'ready' : 'offline'} · ${snapshot?.databases.length ?? 0} DBs`
              : '—'
          }
          percent={sys && sys.apps_total > 0 ? Math.round((sys.apps_running / sys.apps_total) * 100) : 0}
        />
      </div>

      {/* Apps + Live Event Feed */}
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-3">
        <div className="rounded-xl border border-line bg-surface lg:col-span-2">
          <div className="flex items-center justify-between border-b border-line px-4 py-3">
            <div className="flex items-center gap-2 text-xs font-semibold text-fg">
              <Activity className="size-3.5 text-emerald-400" />
              Applications ({snapshot?.apps.length ?? 0})
            </div>
            <div className="flex items-center gap-2">
              <span className="text-[11px] text-muted">
                {snapshot?.projects.length ?? 0} projects
              </span>
              <button
                type="button"
                onClick={onDeployApp}
                className="inline-flex items-center gap-1 rounded-md bg-surface-2 px-2 py-1 text-[11px] font-medium text-fg hover:bg-bg"
              >
                <Plus className="size-3" />
                Deploy
              </button>
            </div>
          </div>
          <div className="divide-y divide-line">
            {(snapshot?.apps ?? []).length === 0 ? (
              <div className="flex flex-col items-center px-4 py-8 text-center text-xs text-muted">
                <p>No applications deployed on this server yet.</p>
                <div className="mt-3 flex flex-wrap justify-center gap-2">
                  <button
                    type="button"
                    onClick={onDeployApp}
                    className="inline-flex items-center gap-1.5 rounded-lg bg-emerald-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-emerald-500"
                  >
                    <Rocket className="size-3.5" />
                    Deploy App
                  </button>
                  <button
                    type="button"
                    onClick={onOpenLocalRepos}
                    className="inline-flex items-center gap-1.5 rounded-lg border border-line bg-surface-2 px-3 py-1.5 text-xs text-fg hover:bg-bg"
                  >
                    <FolderGit2 className="size-3.5" />
                    Browse Local Repos
                  </button>
                </div>
              </div>
            ) : (
              (snapshot?.apps ?? []).map((app) => (
                <button
                  key={app.id}
                  type="button"
                  onClick={() => onOpenApp(app.id)}
                  className="flex w-full items-center gap-3 px-4 py-2.5 text-left transition-colors hover:bg-surface-2/60"
                >
                  <span
                    className={`size-2 shrink-0 rounded-full ${
                      app.status === 'running'
                        ? 'bg-green-400'
                        : app.status === 'deploying' || app.status === 'building'
                          ? 'bg-amber-400'
                          : app.status === 'failed' || app.status === 'crashed'
                            ? 'bg-rose-500'
                            : 'bg-zinc-500'
                    }`}
                  />
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-xs font-medium text-fg">{app.name}</div>
                    <div className="truncate font-mono text-[11px] text-muted">
                      {app.project_name} · {app.source_kind} · port {app.port}
                      {app.git_branch ? ` · ${app.git_branch}` : ''}
                    </div>
                  </div>
                  <span className="rounded bg-surface-2 px-2 py-0.5 font-mono text-[10px] text-muted uppercase">
                    {app.status}
                  </span>
                </button>
              ))
            )}
          </div>
        </div>

        <div className="rounded-xl border border-line bg-surface">
          <div className="flex items-center justify-between border-b border-line px-4 py-3">
            <div className="flex items-center gap-2 text-xs font-semibold text-fg">
              <Radio className="size-3.5 text-emerald-400" />
              Live Event Feed
            </div>
            <span className="font-mono text-[10px] text-muted">SSE /events</span>
          </div>
          <div className="max-h-80 divide-y divide-line overflow-y-auto">
            {(snapshot?.recent_events ?? []).length === 0 ? (
              <div className="px-4 py-8 text-center text-xs text-muted">
                Connected to live event stream. Deploys, crashes, and alerts appear here in real
                time.
              </div>
            ) : (
              [...(snapshot?.recent_events ?? [])].reverse().map((ev) => (
                <div key={ev.id} className="px-4 py-2.5 text-xs">
                  <div className="flex items-center justify-between gap-2">
                    <span className="font-medium text-fg">{ev.title}</span>
                    <span className="shrink-0 font-mono text-[10px] text-muted">
                      {relativeTime(ev.at)}
                    </span>
                  </div>
                  {ev.detail && <p className="mt-0.5 line-clamp-2 text-[11px] text-muted">{ev.detail}</p>}
                  <span className="mt-1 inline-block rounded bg-surface-2 px-1.5 py-0.5 font-mono text-[10px] text-muted">
                    {ev.kind}
                  </span>
                </div>
              ))
            )}
          </div>
        </div>
      </div>

      {/* Databases quick summary */}
      {(snapshot?.databases ?? []).length > 0 && (
        <div className="rounded-xl border border-line bg-surface">
          <div className="flex items-center justify-between border-b border-line px-4 py-2.5 text-xs font-semibold text-fg">
            <div className="flex items-center gap-2">
              <Database className="size-3.5 text-sky-400" />
              Managed Databases ({snapshot?.databases.length ?? 0})
            </div>
            <button
              type="button"
              onClick={onCreateDatabase}
              className="inline-flex items-center gap-1 rounded-md bg-surface-2 px-2 py-1 text-[11px] font-medium text-fg hover:bg-bg"
            >
              <Plus className="size-3" />
              New
            </button>
          </div>
          <div className="grid grid-cols-1 gap-2 p-3 sm:grid-cols-2 lg:grid-cols-3">
            {(snapshot?.databases ?? []).map((db) => (
              <div
                key={db.id}
                className="flex items-center justify-between rounded-lg border border-line bg-bg px-3 py-2 text-xs"
              >
                <div>
                  <div className="font-medium text-fg">{db.name}</div>
                  <div className="font-mono text-[11px] text-muted">
                    {db.engine} {db.version}
                  </div>
                </div>
                <span className="rounded bg-surface-2 px-1.5 py-0.5 font-mono text-[10px] text-muted uppercase">
                  {db.status}
                </span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  )
}

function MetricCard({
  icon,
  label,
  value,
  sub,
  percent,
}: {
  icon: React.ReactNode
  label: string
  value: string
  sub: string
  percent: number
}) {
  return (
    <div className="rounded-xl border border-line bg-surface p-4">
      <div className="flex items-center justify-between text-xs text-muted">
        <span>{label}</span>
        {icon}
      </div>
      <div className="mt-2 font-mono text-2xl font-semibold tracking-tight text-fg tabular-nums">
        {value}
      </div>
      <div className="mt-1 truncate text-[11px] text-muted">{sub}</div>
      <div className="mt-3 h-1.5 overflow-hidden rounded-full bg-bg">
        <div
          className={`h-full rounded-full ${
            percent >= 90 ? 'bg-rose-500' : percent >= 75 ? 'bg-amber-400' : 'bg-emerald-500'
          }`}
          style={{ width: `${Math.min(100, Math.max(2, percent))}%` }}
        />
      </div>
    </div>
  )
}
