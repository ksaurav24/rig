import { useQuery } from '@tanstack/react-query'
import { createFileRoute, Link } from '@tanstack/react-router'
import { Boxes, ChevronRight } from 'lucide-react'
import type { ReactNode } from 'react'

import { appsQuery, databasesQuery, metricsQuery, systemQuery } from '@/api/queries'
import { Page, Panel } from '@/components/app/page'
import { EmptyState, ErrorState, Status, type Tone } from '@/components/app/state'
import { UsagePanel } from '@/components/app/usage'
import { Banner, BannerDescription } from '@/components/fluid/banner'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { bytes, percent, timeAgo } from '@/lib/format'
import { APP_STATUS } from '@/lib/status'
import { cn } from '@/lib/utils'

export const Route = createFileRoute('/_app/')({
  staticData: { crumb: 'Overview' },
  loader: ({ context }) => {
    void context.queryClient.prefetchQuery(systemQuery)
    void context.queryClient.prefetchQuery(appsQuery)
    void context.queryClient.prefetchQuery(metricsQuery({ kind: 'host' }, '1h'))
  },
  component: Overview,
})

/** Attention first: failed and deploying apps above the quietly running ones. */
const ORDER: Record<string, number> = { failed: 0, deploying: 1, running: 2, stopped: 3, idle: 4 }

function Overview() {
  const system = useQuery(systemQuery)
  const apps = useQuery(appsQuery)
  const databases = useQuery(databasesQuery)

  const sorted = [...(apps.data ?? [])].sort(
    (a, b) => ORDER[a.status] - ORDER[b.status] || b.updated_at.localeCompare(a.updated_at),
  )
  const shown = sorted.slice(0, 8)
  const dbRunning = databases.data?.filter((d) => d.status === 'running').length ?? 0
  const s = system.data

  return (
    <Page
      title="Overview"
      actions={
        s ? (
          <span className="font-mono text-[12px] text-muted-foreground tabular-nums">
            Rig v{s.version}
          </span>
        ) : null
      }
    >
      {system.isError ? (
        <Panel>
          <ErrorState error={system.error} onRetry={() => void system.refetch()} />
        </Panel>
      ) : (
        <dl className="grid overflow-hidden rounded-xl bg-card shadow-[var(--shadow-2)] sm:grid-cols-2 lg:grid-cols-4 [&>div]:border-b [&>div]:border-border/80 [&>div]:sm:border-r lg:[&>div]:border-b-0 lg:[&>div:last-child]:border-r-0">
          <Gauge label="CPU" ready={!!s} used={s ? Math.round(s.cpu_percent) : 0} detail="all cores" />
          <Gauge
            label="Memory"
            ready={!!s}
            used={s ? percent(s.memory_used, s.memory_total) : 0}
            detail={s ? `${bytes(s.memory_used)} / ${bytes(s.memory_total)}` : ''}
          />
          <Gauge
            label="Disk"
            ready={!!s}
            used={s ? percent(s.disk_used, s.disk_total) : 0}
            detail={s && s.disk_total > 0 ? `${bytes(s.disk_total - s.disk_used)} free` : 'not available'}
          />
          <div className="flex flex-col justify-center gap-2 px-4 py-3.5 text-[13px]">
            {s ? (
              <>
                <Health tone={s.engine ? 'ok' : 'fail'} label="Engine">
                  {s.engine ? `${s.engine} ${s.engine_version ?? ''}` : 'Unreachable'}
                </Health>
                <Health tone={s.proxy_ready ? 'ok' : 'fail'} label="Proxy">
                  {s.proxy_ready ? 'Caddy serving' : 'Caddy unreachable'}
                </Health>
              </>
            ) : (
              <>
                <Skeleton className="h-4 w-40" />
                <Skeleton className="h-4 w-36" />
              </>
            )}
          </div>
        </dl>
      )}

      {s && (!s.engine || !s.proxy_ready) ? (
        <Banner role="alert" status="error" contrast="high">
          <BannerDescription>
            {!s.engine
              ? 'Rig cannot reach the container engine, so nothing can be deployed. Check that Docker or Podman is running and that Rig may use its socket.'
              : 'Rig cannot reach Caddy, so domains are not being routed. Apps keep running; start Caddy and routes restore on the next deploy.'}
          </BannerDescription>
        </Banner>
      ) : null}

      <UsagePanel target={{ kind: 'host' }} />

      <Panel
        title="Apps"
        description={
          apps.data
            ? `${apps.data.filter((a) => a.status === 'running').length}/${apps.data.length} running` +
              (databases.data?.length ? ` · ${dbRunning}/${databases.data.length} databases` : '')
            : undefined
        }
        actions={
          <Button variant="outline" size="sm" nativeButton={false} render={<Link to="/projects" />}>
            All projects
          </Button>
        }
        flush={apps.isPending || shown.length > 0}
      >
        {apps.isPending ? (
          <div className="flex flex-col gap-3 p-4">
            <Skeleton className="h-5 w-full" />
            <Skeleton className="h-5 w-4/5" />
            <Skeleton className="h-5 w-3/5" />
          </div>
        ) : apps.isError ? (
          <ErrorState error={apps.error} onRetry={() => void apps.refetch()} />
        ) : shown.length === 0 ? (
          <EmptyState
            icon={Boxes}
            title="Nothing deployed yet"
            action={
              <Button nativeButton={false} render={<Link to="/projects" />}>
                Go to projects
              </Button>
            }
          >
            Create a project and add an app from GitHub, a Git URL, or a container image.
          </EmptyState>
        ) : (
          <ul className="divide-y divide-border/80">
            {shown.map((app) => {
              const look = APP_STATUS[app.status]
              return (
                <li key={app.id}>
                  <Link
                    to="/apps/$appId"
                    params={{ appId: app.id }}
                    className="group flex items-center gap-4 px-4 py-2.5 text-[13px] transition-colors outline-none hover:bg-hover focus-visible:bg-hover"
                  >
                    <span className="min-w-0 flex-1 truncate">
                      <span className="font-medium text-foreground">{app.name}</span>
                      <span className="text-muted-foreground"> · {app.project_name}</span>
                    </span>
                    <span className="hidden max-w-60 truncate font-mono text-xs text-muted-foreground md:block">
                      {app.domains[0] ?? ''}
                    </span>
                    <Status tone={look.tone} className="w-24 shrink-0 justify-end">
                      {look.label}
                    </Status>
                    <time
                      dateTime={app.updated_at}
                      className="hidden w-24 shrink-0 text-right text-[12.5px] text-muted-foreground sm:block"
                    >
                      {timeAgo(app.updated_at)}
                    </time>
                    <ChevronRight
                      className="size-4 shrink-0 text-muted-foreground/60 transition-transform group-hover:translate-x-0.5"
                      aria-hidden="true"
                    />
                  </Link>
                </li>
              )
            })}
            {sorted.length > shown.length ? (
              <li className="px-4 py-2.5 text-[13px] text-muted-foreground">
                and {sorted.length - shown.length} more under{' '}
                <Link to="/projects" className="font-medium text-foreground underline-offset-4 hover:underline">
                  Projects
                </Link>
              </li>
            ) : null}
          </ul>
        )}
      </Panel>
    </Page>
  )
}

function Gauge({ label, used, detail, ready }: { label: string; used: number; detail: string; ready: boolean }) {
  const tone = used >= 90 ? 'bg-fail' : used >= 75 ? 'bg-warn' : 'bg-primary'
  return (
    <div className="flex flex-col gap-2 px-4 py-3.5">
      <dt className="text-[12.5px] font-medium text-muted-foreground">{label}</dt>
      <dd className="flex flex-col gap-2">
        {ready ? (
          <>
            <span className="flex items-baseline gap-2">
              <span className="text-2xl font-semibold tracking-tight tabular-nums">{used}%</span>
              <span className="truncate text-[12px] text-muted-foreground">{detail}</span>
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
                style={{ width: `${Math.max(used, 1)}%` }}
              />
            </span>
          </>
        ) : (
          <>
            <Skeleton className="h-8 w-24" />
            <Skeleton className="h-1.5 w-full" />
          </>
        )}
      </dd>
    </div>
  )
}

function Health({ tone, label, children }: { tone: Tone; label: string; children: ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-3">
      <Status tone={tone} className="text-muted-foreground">
        {label}
      </Status>
      <span className="min-w-0 truncate text-[12.5px] font-medium text-foreground">{children}</span>
    </div>
  )
}
