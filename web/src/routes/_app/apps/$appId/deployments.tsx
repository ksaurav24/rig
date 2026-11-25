import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { Play, Undo2 } from 'lucide-react'
import { useEffect } from 'react'

import { api, type Schemas, unwrap } from '@/api/client'
import { deploymentsQuery, meQuery } from '@/api/queries'
import { LogViewer } from '@/components/app/log-viewer'
import { Panel } from '@rig/ui/app/page'
import { EmptyState, ErrorState, Status } from '@/components/app/state'
import { Banner, BannerDescription } from '@rig/ui/banner'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { useLogStream } from '@/hooks/use-log-stream'
import { duration, fullDate, timeAgo } from '@/lib/format'
import { hasRole } from '@/lib/nav'
import { DEPLOYMENT_STATUS, deploymentInProgress, triggerLabel } from '@/lib/status'
import { cn } from '@rig/ui/lib/utils'

type Deployment = Schemas['Deployment']

export const Route = createFileRoute('/_app/apps/$appId/deployments')({
  staticData: { crumb: 'Deployments' },
  validateSearch: (search: Record<string, unknown>): { d?: string } =>
    typeof search.d === 'string' && search.d ? { d: search.d } : {},
  loader: ({ context, params }) => void context.queryClient.prefetchQuery(deploymentsQuery(params.appId)),
  component: Deployments,
})

function Deployments() {
  const { appId } = Route.useParams()
  const { d: selectedId } = Route.useSearch()
  const navigate = useNavigate({ from: Route.fullPath })
  const deployments = useQuery(deploymentsQuery(appId))

  if (deployments.isPending) {
    return (
      <Panel>
        <div className="flex flex-col gap-2">
          <Skeleton className="h-6 w-full" />
          <Skeleton className="h-6 w-4/5" />
          <Skeleton className="h-6 w-3/5" />
        </div>
      </Panel>
    )
  }
  if (deployments.isError) {
    return <ErrorState error={deployments.error} onRetry={() => void deployments.refetch()} />
  }
  if (deployments.data.length === 0) {
    return (
      <Panel>
        <EmptyState icon={Play} title="Not deployed yet">
          Use Deploy above to build and start this app. Output streams here.
        </EmptyState>
      </Panel>
    )
  }

  const selected = deployments.data.find((d) => d.id === selectedId) ?? deployments.data[0]

  return (
    <div className="grid items-start gap-6 lg:grid-cols-[minmax(0,2fr)_minmax(0,3fr)]">
      <Panel flush className="lg:max-h-[calc(100dvh-17rem)] lg:overflow-y-auto">
        <ul className="divide-y divide-border/80">
          {deployments.data.map((d) => {
            const s = DEPLOYMENT_STATUS[d.status]
            const active = d.id === selected.id
            const took = d.started_at ? duration(d.started_at, d.finished_at) : null
            return (
              <li key={d.id}>
                <button
                  type="button"
                  aria-current={active ? 'true' : undefined}
                  onClick={() => void navigate({ search: { d: d.id }, replace: true })}
                  className={cn(
                    'flex w-full flex-col gap-1 px-4 py-3 text-left text-[13px] transition-colors outline-none hover:bg-hover focus-visible:bg-hover',
                    active && 'bg-active hover:bg-active',
                  )}
                >
                  <span className="flex items-center justify-between gap-3">
                    <Status tone={s.tone}>{s.label}</Status>
                    <time
                      dateTime={d.created_at}
                      title={fullDate(d.created_at)}
                      className="shrink-0 text-[12px] text-muted-foreground"
                    >
                      {timeAgo(d.created_at)}
                    </time>
                  </span>
                  <span className="flex items-center justify-between gap-2 text-[12px] text-muted-foreground">
                    <span className="truncate font-mono">
                      {d.commit_sha ? `${d.commit_sha.slice(0, 7)} ` : ''}
                      {d.commit_msg ?? d.image_ref ?? triggerLabel(d.trigger)}
                    </span>
                    <span className="shrink-0 tabular-nums">
                      {triggerLabel(d.trigger)}
                      {took ? ` · ${took}` : ''}
                    </span>
                  </span>
                </button>
              </li>
            )
          })}
        </ul>
      </Panel>

      <DeploymentDetail key={selected.id} deployment={selected} />
    </div>
  )
}

function DeploymentDetail({ deployment: d }: { deployment: Deployment }) {
  const { data: me } = useQuery(meQuery)
  const queryClient = useQueryClient()
  const navigate = useNavigate({ from: Route.fullPath })
  const { lines, state, result } = useLogStream(`/api/v1/deployments/${d.id}/logs`)
  const inProgress = deploymentInProgress(d.status)
  const s = DEPLOYMENT_STATUS[d.status]

  useEffect(() => {
    if (result) void queryClient.invalidateQueries({ queryKey: ['apps'] })
  }, [result, queryClient])

  const rollback = useMutation({
    mutationFn: () =>
      unwrap(api.POST('/api/v1/deployments/{id}/rollback', { params: { path: { id: d.id } } })),
    meta: { success: 'Rollback started' },
    onSuccess: async (next) => {
      await queryClient.invalidateQueries({ queryKey: ['apps'] })
      await navigate({ search: { d: next.id }, replace: true })
    },
  })

  const canRollback = d.status === 'superseded' && d.image_ref && me && hasRole(me.role, 'developer')

  return (
    <div className="flex min-w-0 flex-col gap-4">
      <div className="flex flex-wrap items-start justify-between gap-4 rounded-xl bg-card px-4 py-3 shadow-[var(--shadow-2)]">
        <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1 text-[13px]">
          <dt className="text-muted-foreground">Trigger</dt>
          <dd className="font-medium text-foreground">{triggerLabel(d.trigger)}</dd>
          {d.commit_sha ? (
            <>
              <dt className="text-muted-foreground">Commit</dt>
              <dd className="truncate">
                <span className="font-mono text-xs">{d.commit_sha.slice(0, 7)}</span>
                {d.commit_msg ? <span className="text-muted-foreground"> {d.commit_msg}</span> : null}
              </dd>
            </>
          ) : null}
          {d.image_ref ? (
            <>
              <dt className="text-muted-foreground">Image</dt>
              <dd className="truncate font-mono text-xs leading-5">{d.image_ref}</dd>
            </>
          ) : null}
          <dt className="text-muted-foreground">Duration</dt>
          <dd className="tabular-nums">
            {d.started_at ? duration(d.started_at, d.finished_at) : 'Queued'}
          </dd>
        </dl>
        {canRollback ? (
          <Button variant="outline" size="sm" onClick={() => rollback.mutate()} disabled={rollback.isPending}>
            {rollback.isPending ? <Spinner data-icon="inline-start" /> : <Undo2 data-icon="inline-start" />}
            Roll back to this
          </Button>
        ) : null}
      </div>

      {d.error ? (
        <Banner role="alert" status="error" contrast="high">
          <BannerDescription>{d.error}</BannerDescription>
        </Banner>
      ) : null}

      <LogViewer
        lines={lines}
        className="h-[calc(100dvh-27rem)] min-h-72"
        status={
          inProgress && state !== 'ended' ? (
            <Status tone="busy">{s.label}</Status>
          ) : state === 'error' ? (
            <Status tone="fail">Could not load output</Status>
          ) : (
            <Status tone={s.tone}>{s.label}</Status>
          )
        }
        empty={inProgress ? 'Waiting for output.' : 'No build output recorded.'}
      />
    </div>
  )
}
