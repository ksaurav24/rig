import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute, Outlet, useLocation, useNavigate } from '@tanstack/react-router'
import { ExternalLink, GitBranch, Package, Play, RotateCw, Square } from 'lucide-react'
import { useState } from 'react'

import { api, unwrap } from '@/api/client'
import { appQuery, meQuery } from '@/api/queries'
import { ErrorState, Status } from '@/components/app/state'
import { Tabs, TabsItem } from '@/components/fluid/tabs'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { hasRole } from '@/lib/nav'
import { APP_STATUS } from '@/lib/status'

export const Route = createFileRoute('/_app/apps/$appId')({
  loader: async ({ context, params }) => {
    const app = await context.queryClient.ensureQueryData(appQuery(params.appId))
    return {
      crumb: app.name,
      parents: [
        { label: 'Projects', to: '/projects' },
        { label: app.project_name, to: `/projects/${app.project_id}` },
      ],
    }
  },
  errorComponent: ({ error, reset }) => (
    <div className="mx-auto w-full max-w-6xl px-4 py-6 md:px-8 md:py-8">
      <ErrorState error={error} onRetry={reset} />
    </div>
  ),
  component: AppLayout,
})

const TABS = [
  { to: '/apps/$appId', suffix: '', label: 'Overview' },
  { to: '/apps/$appId/deployments', suffix: '/deployments', label: 'Deployments' },
  { to: '/apps/$appId/logs', suffix: '/logs', label: 'Logs' },
  { to: '/apps/$appId/terminal', suffix: '/terminal', label: 'Terminal' },
  { to: '/apps/$appId/environment', suffix: '/environment', label: 'Environment' },
  { to: '/apps/$appId/domains', suffix: '/domains', label: 'Domains' },
  { to: '/apps/$appId/settings', suffix: '/settings', label: 'Settings' },
] as const

function AppLayout() {
  const { appId } = Route.useParams()
  const { data: me } = useQuery(meQuery)
  const { data: app } = useQuery(appQuery(appId))
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const [confirmStop, setConfirmStop] = useState(false)
  const canDeploy = me ? hasRole(me.role, 'developer') : false
  const { pathname } = useLocation()

  const refresh = () => queryClient.invalidateQueries({ queryKey: ['apps'] })

  const deploy = useMutation({
    mutationFn: () => unwrap(api.POST('/api/v1/apps/{id}/deploy', { params: { path: { id: appId } } })),
    meta: { success: 'Deployment started' },
    onSuccess: async () => {
      await refresh()
      await navigate({ to: '/apps/$appId/deployments', params: { appId } })
    },
  })

  const stop = useMutation({
    mutationFn: () => unwrap(api.POST('/api/v1/apps/{id}/stop', { params: { path: { id: appId } } })),
    meta: { success: 'App stopped' },
    onSuccess: () => {
      setConfirmStop(false)
      return refresh()
    },
  })

  if (!app) {
    return (
      <div className="mx-auto flex w-full max-w-6xl flex-col gap-4 px-4 py-6 md:px-8 md:py-8">
        <Skeleton className="h-8 w-56" />
        <Skeleton className="h-9 w-full" />
        <Skeleton className="h-48 w-full" />
      </div>
    )
  }

  const status = APP_STATUS[app.status]
  const busy = app.status === 'deploying'
  const url = app.domains[0]
  const SourceIcon = app.source_kind === 'git' ? GitBranch : Package
  const basePath = `/apps/${appId}`
  const selectedIndex = Math.max(
    0,
    TABS.findIndex((t) =>
      t.suffix === ''
        ? pathname === basePath || pathname === `${basePath}/`
        : pathname.startsWith(`${basePath}${t.suffix}`),
    ),
  )

  return (
    <div className="mx-auto flex w-full max-w-6xl flex-col gap-6 px-4 py-6 md:px-8 md:py-8">
      <header className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
        <div className="flex min-w-0 flex-col gap-1.5">
          <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
            <h1 className="truncate text-xl font-semibold tracking-tight">{app.name}</h1>
            <Status tone={status.tone} variant="solid">
              {status.label}
            </Status>
          </div>
          <div className="flex min-w-0 flex-wrap items-center gap-x-4 gap-y-1 text-[13px] text-muted-foreground">
            <span className="flex min-w-0 items-center gap-1.5">
              <SourceIcon className="size-3.5 shrink-0" aria-hidden />
              <span className="truncate font-mono text-xs">
                {app.source_kind === 'git' ? app.git_url : app.image}
              </span>
            </span>
            {url ? (
              <a
                href={`https://${url}`}
                target="_blank"
                rel="noreferrer"
                className="flex items-center gap-1 text-foreground underline-offset-4 hover:underline"
              >
                {url}
                <ExternalLink className="size-3" aria-hidden />
              </a>
            ) : null}
          </div>
        </div>
        {canDeploy ? (
          <div className="flex shrink-0 items-center gap-2">
            {app.status === 'running' ? (
              <Button variant="outline" onClick={() => setConfirmStop(true)}>
                <Square data-icon="inline-start" />
                Stop
              </Button>
            ) : null}
            <Button onClick={() => deploy.mutate()} disabled={busy || deploy.isPending}>
              {busy || deploy.isPending ? (
                <Spinner data-icon="inline-start" />
              ) : app.current_deployment_id ? (
                <RotateCw data-icon="inline-start" />
              ) : (
                <Play data-icon="inline-start" />
              )}
              {busy ? 'Deploying' : app.current_deployment_id ? 'Redeploy' : 'Deploy'}
            </Button>
          </div>
        ) : null}
      </header>

      <div className="-mx-4 overflow-x-auto px-4 md:mx-0 md:px-0">
        <Tabs
          selectedIndex={selectedIndex}
          onSelect={(i) => void navigate({ to: TABS[i].to, params: { appId } })}
          aria-label="App sections"
          className="w-fit border border-border/80 bg-card/70"
        >
          {TABS.map((tab, i) => (
            <TabsItem key={tab.to} index={i} label={tab.label} />
          ))}
        </Tabs>
      </div>

      <Outlet />

      <AlertDialog open={confirmStop} onOpenChange={setConfirmStop}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Stop {app.name}?</AlertDialogTitle>
            <AlertDialogDescription>
              The container is removed and domains stop answering until the next deploy. Settings and history stay intact.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Keep running</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              disabled={stop.isPending}
              onClick={(e) => {
                e.preventDefault()
                stop.mutate()
              }}
            >
              {stop.isPending ? <Spinner data-icon="inline-start" /> : null}
              Stop app
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
