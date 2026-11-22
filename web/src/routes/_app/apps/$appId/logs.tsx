import { useQuery } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { ScrollText } from 'lucide-react'

import { appQuery } from '@/api/queries'
import { LogViewer } from '@/components/app/log-viewer'
import { Panel } from '@/components/app/page'
import { EmptyState, Status } from '@/components/app/state'
import { useLogStream } from '@/hooks/use-log-stream'

export const Route = createFileRoute('/_app/apps/$appId/logs')({
  staticData: { crumb: 'Logs' },
  component: AppLogs,
})

function AppLogs() {
  const { appId } = Route.useParams()
  const { data: app } = useQuery(appQuery(appId))
  const running = app?.status === 'running'
  // Keyed on the deployment so a redeploy reconnects to the new container.
  const url = running ? `/api/v1/apps/${appId}/logs?d=${app.current_deployment_id ?? ''}` : null
  const { lines, state } = useLogStream(url, { replays: true })

  if (!app) return null
  if (!running) {
    return (
      <Panel>
        <EmptyState icon={ScrollText} title="No container to read from">
          Logs appear here while the app is running. Build output is under Deployments.
        </EmptyState>
      </Panel>
    )
  }

  return (
    <LogViewer
      lines={lines}
      className="h-[calc(100dvh-19rem)] min-h-80"
      status={
        state === 'live' ? (
          <Status tone="ok">Live</Status>
        ) : state === 'error' ? (
          <Status tone="fail">Disconnected</Status>
        ) : (
          <Status tone="busy">Connecting</Status>
        )
      }
      empty="The container has not written anything yet."
    />
  )
}
