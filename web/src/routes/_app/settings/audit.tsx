import { useQuery } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { ScrollText, Search, X } from 'lucide-react'
import { useDeferredValue, useMemo, useState } from 'react'
import { auditQuery } from '@/api/queries'
import { Page, Panel } from '@/components/app/page'
import { EmptyState, ErrorState } from '@/components/app/state'
import { Badge } from '@/components/fluid/badge'
import { Input } from '@/components/fluid/input'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/fluid/table'
import { Tooltip } from '@/components/fluid/tooltip'
import { Skeleton } from '@/components/ui/skeleton'
import { fullDate, timeAgo } from '@/lib/format'

export const Route = createFileRoute('/_app/settings/audit')({
  staticData: { crumb: 'Audit log' },
  loader: ({ context }) => void context.queryClient.prefetchQuery(auditQuery),
  component: AuditPage,
})

function actionColor(action: string): 'red' | 'emerald' | 'amber' | 'blue' | 'gray' {
  if (action.includes('delete') || action.includes('revoke') || action.includes('disable')) return 'red'
  if (action.includes('deploy') || action.includes('create') || action.includes('enable')) return 'emerald'
  if (action.includes('reveal') || action.includes('rotate') || action.includes('reset')) return 'amber'
  if (action.startsWith('auth.')) return 'blue'
  return 'gray'
}

function AuditPage() {
  const audit = useQuery(auditQuery)
  const [query, setQuery] = useState('')
  const needle = useDeferredValue(query.trim().toLowerCase())

  const filtered = useMemo(() => {
    const list = audit.data ?? []
    if (!needle) return list
    return list.filter((e) =>
      `${e.action} ${e.actor} ${e.target ?? ''} ${e.detail ?? ''} ${e.ip ?? ''}`.toLowerCase().includes(needle),
    )
  }, [audit.data, needle])

  return (
    <Page
      title="Audit log"
      description="Latest 200 security and deployment events."
      actions={
        audit.data?.length ? (
          <div className="relative w-56 sm:w-64">
            <Search className="pointer-events-none absolute top-1/2 left-2.5 z-10 size-3.5 -translate-y-1/2 text-muted-foreground" />
            <Input
              size="compact"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Filter events"
              aria-label="Filter audit events"
              className="pl-8 pr-7"
            />
            {query ? (
              <button
                type="button"
                onClick={() => setQuery('')}
                aria-label="Clear filter"
                className="absolute top-1/2 right-2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
              >
                <X className="size-3.5" />
              </button>
            ) : null}
          </div>
        ) : undefined
      }
    >
      {audit.isPending ? (
        <Panel flush>
          <div className="space-y-2 p-4">
            {Array.from({ length: 6 }, (_, i) => (
              <Skeleton key={i} className="h-8" />
            ))}
          </div>
        </Panel>
      ) : audit.isError ? (
        <ErrorState error={audit.error} onRetry={() => void audit.refetch()} />
      ) : audit.data.length === 0 ? (
        <EmptyState icon={ScrollText} title="Nothing recorded yet">
          Organization actions will appear here.
        </EmptyState>
      ) : filtered.length === 0 ? (
        <EmptyState icon={Search} title="No events match">
          Try a different action name, actor email, or IP address.
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
              {filtered.map((e, index) => {
                const fullTarget = [e.target, e.detail ? `(${e.detail})` : null].filter(Boolean).join(' ')
                return (
                  <TableRow key={e.id} index={index}>
                    <TableCell className="pl-4">
                      <Badge variant="solid" color={actionColor(e.action)} size="compact" className="font-mono">
                        {e.action}
                      </Badge>
                    </TableCell>
                    <TableCell className="max-w-60 text-muted-foreground">
                      {fullTarget ? (
                        <Tooltip content={fullTarget}>
                          <span className="block truncate font-mono text-[12px]">{fullTarget}</span>
                        </Tooltip>
                      ) : (
                        '–'
                      )}
                    </TableCell>
                    <TableCell className="max-w-48 truncate text-foreground">{e.actor}</TableCell>
                    <TableCell className="tabular hidden font-mono text-[12px] text-muted-foreground md:table-cell">
                      {e.ip ?? '–'}
                    </TableCell>
                    <TableCell className="pr-4 text-right whitespace-nowrap text-muted-foreground">
                      <Tooltip content={fullDate(e.created_at)}>
                        <time dateTime={e.created_at}>{timeAgo(e.created_at)}</time>
                      </Tooltip>
                    </TableCell>
                  </TableRow>
                )
              })}
            </TableBody>
          </Table>
        </Panel>
      )}
    </Page>
  )
}
