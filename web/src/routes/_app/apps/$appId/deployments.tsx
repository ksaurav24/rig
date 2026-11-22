import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { Rocket, Undo2 } from 'lucide-react'
import { useEffect } from 'react'

import { api, type Schemas, unwrap } from '@/api/client'
import { deploymentsQuery, meQuery } from '@/api/queries'
import { LogViewer } from '@/components/app/log-viewer'
import { Panel } from '@/components/app/page'
import { EmptyState, ErrorState, Status } from '@/components/app/state'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { useLogStream } from '@/hooks/use-log-stream'
import { duration, fullDate, timeAgo } from '@/lib/format'
import { hasRole } from '@/lib/nav'
import { DEPLOYMENT_STATUS, deploymentInProgress, triggerLabel } from '@/lib/status'
import { cn } from '@/lib/utils'

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
        <EmptyState icon={Rocket} title="Not deployed yet">
          Press Deploy to build and start this app. Its output streams here.
        </EmptyState>
      </Panel>
    )
  }

  // Default to the newest, which is the one in progress if there is one.
  const selected = deployments.data.find((d) => d.id === selectedId) ?? deployments.data[0]

  return (
    <div className="grid items-start gap-6 lg:grid-cols-[minmax(0,2fr)_minmax(0,3fr)]">
      <Panel flush className="lg:max-h-[calc(100dvh-17rem)] lg:overflow-y-auto">
        <ul className="divide-y">
          {deployments.data.map((d) => {
            const s = DEPLOYMENT_STATUS[d.status]
            const active = d.id === selected.id
            return (
              <li key={d.id}>
                <button
                  type="button"
                  aria-current={active ? 'true' : undefined}
                  onClick={() => void navigate({ search: { d: d.id }, replace: true })}
                  className={cn(
                    'flex w-full flex-col gap-1 px-4 py-2.5 text-left text-[0.8125rem] transition-colors outline-none hover:bg-muted/50 focus-visible:bg-muted/50',
                    active && 'bg-muted/70 hover:bg-muted/70',
                  )}
                >
                  <span className="flex items-center justify-between gap-3">
                    <Status tone={s.tone}>{s.label}</Status>
                    <time
                      dateTime={d.created_at}
                      title={fullDate(d.created_at)}
                      className="shrink-0 text-muted-foreground"
                    >
                      {timeAgo(d.created_at)}
                    </time>
                  </span>
                  <span className="truncate text-muted-foreground">
                    {d.commit_msg ?? d.image_ref ?? triggerLabel(d.trigger)}
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

  // The stream's end event arrives before the next poll would notice.
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
      <div className="flex flex-wrap items-start justify-between gap-3">
        <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1 text-[0.8125rem]">
          <dt className="text-muted-foreground">Started by</dt>
          <dd>{triggerLabel(d.trigger)}</dd>
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
          <dt className="text-muted-foreground">Took</dt>
          <dd className="tabular-nums">
            {d.started_at ? duration(d.started_at, d.finished_at) : 'Waiting in the queue'}
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
        <p role="alert" className="rounded-md border border-fail/30 bg-fail/5 px-3 py-2 text-[0.8125rem] text-fail">
          {d.error}
        </p>
      ) : null}

      <LogViewer
        lines={lines}
        className="h-[calc(100dvh-26rem)] min-h-72"
        status={
          inProgress && state !== 'ended' ? (
            <Status tone="busy">{s.label}</Status>
          ) : state === 'error' ? (
            <Status tone="fail">Could not load output</Status>
          ) : (
            <Status tone={s.tone}>{s.label}</Status>
          )
        }
        empty={inProgress ? 'Waiting for output.' : 'This deployment produced no output.'}
      />
    </div>
  )
}
