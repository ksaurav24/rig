import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute, Link, Outlet, useLocation, useNavigate } from '@tanstack/react-router'
import { ExternalLink, GitBranch, Package, Rocket, Square } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'

import { api, unwrap } from '@/api/client'
import { appQuery, meQuery } from '@/api/queries'
import { ErrorState, Status } from '@/components/app/state'
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
  { to: '/apps/$appId', label: 'Overview', exact: true },
  { to: '/apps/$appId/deployments', label: 'Deployments', exact: false },
  { to: '/apps/$appId/logs', label: 'Logs', exact: false },
  { to: '/apps/$appId/terminal', label: 'Terminal', exact: false },
  { to: '/apps/$appId/environment', label: 'Environment', exact: false },
  { to: '/apps/$appId/domains', label: 'Domains', exact: false },
  { to: '/apps/$appId/settings', label: 'Settings', exact: false },
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
  const tabsRef = useRef<HTMLElement>(null)
  const loaded = app !== undefined
  // On narrow screens the tab strip scrolls; keep the current tab in view.
  useEffect(() => {
    tabsRef.current
      ?.querySelector('[aria-current=page]')
      ?.scrollIntoView({ block: 'nearest', inline: 'nearest' })
  }, [pathname, loaded])

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

  return (
    <div className="mx-auto flex w-full max-w-6xl flex-col gap-6 px-4 py-6 md:px-8 md:py-8">
      <header className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
        <div className="flex min-w-0 flex-col gap-1.5">
          <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
            <h1 className="truncate text-xl font-semibold tracking-tight">{app.name}</h1>
            <Status tone={status.tone}>{status.label}</Status>
          </div>
          <div className="flex min-w-0 flex-wrap items-center gap-x-4 gap-y-1 text-[0.8125rem] text-muted-foreground">
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
              {busy || deploy.isPending ? <Spinner data-icon="inline-start" /> : <Rocket data-icon="inline-start" />}
              {busy ? 'Deploying' : app.current_deployment_id ? 'Redeploy' : 'Deploy'}
            </Button>
          </div>
        ) : null}
      </header>

      <nav
        ref={tabsRef}
        aria-label="App sections"
        className="-mx-4 [scrollbar-width:none] overflow-x-auto border-b px-4 [mask-image:linear-gradient(to_right,transparent,#000_1rem,#000_calc(100%-2rem),transparent)] md:-mx-8 md:px-8 md:[mask-image:none]"
      >
        <ul className="flex gap-5 pr-4">
          {TABS.map((tab) => (
            <li key={tab.to}>
              <Link
                to={tab.to}
                params={{ appId }}
                activeOptions={{ exact: tab.exact }}
                activeProps={{ 'aria-current': 'page' }}
                className="-mb-px block border-b-2 border-transparent py-2.5 text-[0.8125rem] font-medium whitespace-nowrap text-muted-foreground transition-colors hover:text-foreground aria-[current=page]:border-primary aria-[current=page]:text-foreground"
              >
                {tab.label}
              </Link>
            </li>
          ))}
        </ul>
      </nav>

      <Outlet />

      <AlertDialog open={confirmStop} onOpenChange={setConfirmStop}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Stop {app.name}?</AlertDialogTitle>
            <AlertDialogDescription>
              Its container is removed and its domains stop answering until you deploy again. Settings,
              variables and deployment history are kept.
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
