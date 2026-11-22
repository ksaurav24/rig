import { useQuery } from '@tanstack/react-query'
import { createFileRoute, Link } from '@tanstack/react-router'
import { Globe, Rocket, TriangleAlert } from 'lucide-react'
import type { ReactNode } from 'react'

import type { Schemas } from '@/api/client'
import { appQuery, deploymentsQuery } from '@/api/queries'
import { CopyButton } from '@/components/app/copy-button'
import { Panel } from '@/components/app/page'
import { EmptyState, Status } from '@/components/app/state'
import { UsagePanel } from '@/components/app/usage'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { duration, fullDate, timeAgo } from '@/lib/format'
import { DEPLOYMENT_STATUS, triggerLabel } from '@/lib/status'

export const Route = createFileRoute('/_app/apps/$appId/')({
  loader: ({ context, params }) => void context.queryClient.prefetchQuery(deploymentsQuery(params.appId)),
  component: AppOverview,
})

function Fact({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex items-baseline justify-between gap-4 py-2 text-[0.8125rem]">
      <dt className="shrink-0 text-muted-foreground">{label}</dt>
      <dd className="min-w-0 truncate text-right">{children}</dd>
    </div>
  )
}

function AppOverview() {
  const { appId } = Route.useParams()
  const { data: app } = useQuery(appQuery(appId))
  const deployments = useQuery(deploymentsQuery(appId))
  if (!app) return null

  const recent = deployments.data?.slice(0, 5) ?? []
  const local = app.host_port ? `http://127.0.0.1:${app.host_port}` : null

  return (
    <div className="grid gap-6 lg:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]">
      <div className="flex min-w-0 flex-col gap-6">
        {app.last_crash ? <CrashNotice appId={appId} crash={app.last_crash} failed={app.status === 'failed'} /> : null}
        <Panel
          title="Reach it"
          description="Where this app answers once it is running."
          actions={
            <Button
              variant="outline"
              size="sm"
              nativeButton={false}
              render={<Link to="/apps/$appId/domains" params={{ appId }} />}
            >
              Manage domains
            </Button>
          }
          flush={app.domains.length > 0 || local !== null}
        >
          {app.domains.length === 0 && !local ? (
            <EmptyState icon={Globe} title="No address yet">
              Deploy the app, then add a domain to put it on the internet.
            </EmptyState>
          ) : (
            <ul className="divide-y">
              {app.domains.map((host) => (
                <li key={host} className="flex items-center justify-between gap-3 px-4 py-2.5">
                  <a
                    href={`https://${host}`}
                    target="_blank"
                    rel="noreferrer"
                    className="truncate font-mono text-xs underline-offset-4 hover:underline"
                  >
                    {host}
                  </a>
                  <CopyButton value={`https://${host}`} label="Copy URL" />
                </li>
              ))}
              {local ? (
                <li className="flex items-center justify-between gap-3 px-4 py-2.5">
                  <span className="flex min-w-0 items-baseline gap-2">
                    <span className="truncate font-mono text-xs">{local}</span>
                    <span className="shrink-0 text-xs text-muted-foreground">on the server only</span>
                  </span>
                  <CopyButton value={local} label="Copy URL" />
                </li>
              ) : null}
            </ul>
          )}
        </Panel>

        {app.current_deployment_id ? <UsagePanel target={{ kind: 'app', id: appId }} /> : null}

        <Panel
          title="Recent deployments"
          actions={
            <Button
              variant="outline"
              size="sm"
              nativeButton={false}
              render={<Link to="/apps/$appId/deployments" params={{ appId }} />}
            >
              View all
            </Button>
          }
          flush={deployments.isPending || recent.length > 0}
        >
          {deployments.isPending ? (
            <div className="flex flex-col gap-2 p-4">
              <Skeleton className="h-5 w-full" />
              <Skeleton className="h-5 w-4/5" />
            </div>
          ) : recent.length === 0 ? (
            <EmptyState icon={Rocket} title="Not deployed yet">
              Press Deploy to build and start this app.
            </EmptyState>
          ) : (
            <ul className="divide-y">
              {recent.map((d) => {
                const s = DEPLOYMENT_STATUS[d.status]
                return (
                  <li key={d.id}>
                    <Link
                      to="/apps/$appId/deployments"
                      params={{ appId }}
                      search={{ d: d.id }}
                      className="flex items-center gap-3 px-4 py-2.5 text-[0.8125rem] transition-colors hover:bg-muted/50"
                    >
                      <Status tone={s.tone} className="w-24 shrink-0">
                        {s.label}
                      </Status>
                      <span className="min-w-0 flex-1 truncate text-muted-foreground">
                        {d.commit_msg ?? d.image_ref ?? triggerLabel(d.trigger)}
                      </span>
                      <span className="hidden shrink-0 text-muted-foreground tabular-nums sm:inline">
                        {d.started_at ? duration(d.started_at, d.finished_at) : ''}
                      </span>
                      <time
                        dateTime={d.created_at}
                        title={fullDate(d.created_at)}
                        className="w-20 shrink-0 text-right text-muted-foreground"
                      >
                        {timeAgo(d.created_at)}
                      </time>
                    </Link>
                  </li>
                )
              })}
            </ul>
          )}
        </Panel>
      </div>

      <Panel title="Configuration" description="Applied on the next deployment." className="self-start">
        <dl className="-my-2 divide-y">
          <Fact label="Project">{app.project_name}</Fact>
          <Fact label="Source">{app.source_kind === 'git' ? 'Git repository' : 'Container image'}</Fact>
          {app.source_kind === 'git' ? (
            <>
              <Fact label="Branch">{app.git_branch ?? 'Default branch'}</Fact>
              <Fact label="Dockerfile">
                <span className="font-mono text-xs">{app.dockerfile_path}</span>
              </Fact>
            </>
          ) : null}
          <Fact label="Container port">
            <span className="tabular-nums">{app.port}</span>
          </Fact>
          <Fact label="Health check">
            {app.health_path ? <span className="font-mono text-xs">{app.health_path}</span> : 'Port answers'}
          </Fact>
          <Fact label="Memory limit">{app.memory_mb ? `${app.memory_mb} MB` : '512 MB (default)'}</Fact>
          <Fact label="CPU limit">{app.cpu_millis ? `${app.cpu_millis / 1000} CPU` : 'None'}</Fact>
          <Fact label="Persistent paths">
            {app.volumes.length ? (
              <span className="font-mono text-xs">{app.volumes.join(', ')}</span>
            ) : (
              'None, deploys swap with no downtime'
            )}
          </Fact>
          <Fact label="Created">
            <time dateTime={app.created_at} title={fullDate(app.created_at)}>
              {timeAgo(app.created_at)}
            </time>
          </Fact>
        </dl>
      </Panel>
    </div>
  )
}

function CrashNotice({ appId, crash, failed }: { appId: string; crash: Schemas['AppCrash']; failed: boolean }) {
  return (
    <div role="alert" className="flex items-start gap-3 rounded-lg border border-fail/40 bg-fail/8 p-4 text-sm">
      <TriangleAlert aria-hidden className="mt-0.5 size-4 shrink-0 text-fail" />
      <div className="min-w-0 space-y-1">
        <p className="font-medium">
          {failed ? 'The app is down' : 'The app crashed and was restarted'}
          <span className="font-normal text-muted-foreground">
            {' '}
            · <time dateTime={crash.at} title={fullDate(crash.at)}>{timeAgo(crash.at)}</time>
            {crash.restarts > 0 ? ` · ${crash.restarts} restart${crash.restarts === 1 ? '' : 's'}` : null}
          </span>
        </p>
        <p className="text-muted-foreground">It {crash.reason}.</p>
        <p className="flex flex-wrap gap-x-4 gap-y-1">
          <Link to="/apps/$appId/logs" params={{ appId }} className="font-medium underline-offset-4 hover:underline">
            Open the logs
          </Link>
          <Link to="/apps/$appId/settings" params={{ appId }} className="font-medium underline-offset-4 hover:underline">
            Adjust limits
          </Link>
        </p>
      </div>
    </div>
  )
}
