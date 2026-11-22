import { useQuery } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { ScrollText } from 'lucide-react'
import { auditQuery } from '@/api/queries'
import { Page, Panel } from '@/components/app/page'
import { EmptyState, ErrorState } from '@/components/app/state'
import { Skeleton } from '@/components/ui/skeleton'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { fullDate, timeAgo } from '@/lib/format'

export const Route = createFileRoute('/_app/settings/audit')({
  staticData: { crumb: 'Audit log' },
  loader: ({ context }) => void context.queryClient.prefetchQuery(auditQuery),
  component: AuditPage,
})

function AuditPage() {
  const audit = useQuery(auditQuery)
  return (
    <Page title="Audit log" description="Sign-ins, deployments, secret reveals and permission changes. The latest 200 entries.">
      {audit.isPending ? (
        <Panel flush>
          <div className="space-y-px p-4">
            {Array.from({ length: 6 }, (_, i) => (
              <Skeleton key={i} className="h-8" />
            ))}
          </div>
        </Panel>
      ) : audit.isError ? (
        <ErrorState error={audit.error} onRetry={() => void audit.refetch()} />
      ) : audit.data.length === 0 ? (
        <EmptyState icon={ScrollText} title="Nothing recorded yet">
          Actions taken in this organization will be listed here.
        </EmptyState>
      ) : (
        <Panel flush>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="pl-4">Action</TableHead>
                <TableHead>Target</TableHead>
                <TableHead>By</TableHead>
                <TableHead className="hidden md:table-cell">From</TableHead>
                <TableHead className="pr-4 text-right">When</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {audit.data.map((e) => (
                <TableRow key={e.id}>
                  <TableCell className="pl-4">
                    <code className="text-[0.8125rem]">{e.action}</code>
                  </TableCell>
                  <TableCell className="max-w-56 truncate text-muted-foreground">
                    {e.target ?? '–'}
                    {e.detail ? <span className="ml-1.5">({e.detail})</span> : null}
                  </TableCell>
                  <TableCell className="max-w-48 truncate">{e.actor}</TableCell>
                  <TableCell className="tabular hidden text-muted-foreground md:table-cell">{e.ip ?? '–'}</TableCell>
                  <TableCell className="pr-4 text-right whitespace-nowrap text-muted-foreground">
                    <Tooltip>
                      <TooltipTrigger render={<time dateTime={e.created_at} />}>{timeAgo(e.created_at)}</TooltipTrigger>
                      <TooltipContent>{fullDate(e.created_at)}</TooltipContent>
                    </Tooltip>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </Panel>
      )}
    </Page>
  )
}
