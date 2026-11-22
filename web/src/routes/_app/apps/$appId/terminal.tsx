import { useQuery } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { SquareTerminal } from 'lucide-react'
import { lazy, Suspense } from 'react'

import { appQuery, meQuery } from '@/api/queries'
import { Panel } from '@/components/app/page'
import { EmptyState } from '@/components/app/state'
import { Skeleton } from '@/components/ui/skeleton'
import { hasRole } from '@/lib/nav'

// xterm is the heaviest thing in the dashboard; only this tab pays for it.
const Terminal = lazy(() => import('@/components/app/terminal'))

export const Route = createFileRoute('/_app/apps/$appId/terminal')({
  staticData: { crumb: 'Terminal' },
  component: AppTerminal,
})

const SIZE = 'h-[calc(100dvh-19rem)] min-h-80'

function AppTerminal() {
  const { appId } = Route.useParams()
  const { data: me } = useQuery(meQuery)
  const { data: app } = useQuery(appQuery(appId))

  if (!app || !me) return null
  if (!hasRole(me.role, 'developer')) {
    return (
      <Panel>
        <EmptyState icon={SquareTerminal} title="Your role cannot open a shell">
          A shell can read the app's secrets, so it needs the developer role or above. Ask an admin to change yours.
        </EmptyState>
      </Panel>
    )
  }
  if (app.status !== 'running') {
    return (
      <Panel>
        <EmptyState icon={SquareTerminal} title="No container to connect to">
          Deploy the app, then open a shell in its running container here.
        </EmptyState>
      </Panel>
    )
  }

  return (
    <div className="flex flex-col gap-2">
      <Suspense fallback={<Skeleton className={SIZE} />}>
        {/* Keyed on the deployment: a redeploy replaces the container, so start a fresh shell. */}
        <Terminal key={app.current_deployment_id} appId={appId} className={SIZE} />
      </Suspense>
      <p className="text-xs text-muted-foreground">
        Changes made here live only in this container and are gone after the next deployment. Every shell is recorded
        in the audit log.
      </p>
    </div>
  )
}
