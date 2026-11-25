import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Link, createFileRoute } from '@tanstack/react-router'
import { Archive, Database as DatabaseIcon, Ellipsis, Eye, EyeOff, KeyRound, Play, Plus, Square, Trash2 } from 'lucide-react'
import { useState } from 'react'
import { api, unwrap, type Schemas } from '@/api/client'
import { databaseEnginesQuery, databasesQuery, meQuery, projectsQuery } from '@/api/queries'
import { BackupsSheet } from '@/components/app/backups-sheet'
import { CopyButton } from '@/components/app/copy-button'
import { NewDatabaseDialog } from '@/components/app/new-database-dialog'
import { Page, Panel } from '@rig/ui/app/page'
import { EngineMark } from '@/components/app/service-icons'
import { EmptyState, ErrorState, Status } from '@/components/app/state'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@rig/ui/dialog'
import {
  DropdownContent,
  DropdownMenu,
  DropdownSeparator,
  DropdownTrigger,
  MenuItem,
} from '@rig/ui/dropdown'
import { Field, FieldLabel } from '@rig/ui/field'
import { Input } from '@rig/ui/input'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@rig/ui/table'
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
import { timeAgo } from '@/lib/format'
import { hasRole } from '@/lib/nav'
import { DATABASE_STATUS } from '@/lib/status'

type Database = Schemas['Database']
type Engine = Schemas['DatabaseEngine']

export const Route = createFileRoute('/_app/databases/')({
  staticData: { crumb: 'Databases' },
  loader: ({ context }) => {
    void context.queryClient.prefetchQuery(databasesQuery)
    void context.queryClient.prefetchQuery(databaseEnginesQuery)
  },
  component: DatabasesPage,
})

function DatabasesPage() {
  const databases = useQuery(databasesQuery)
  const engines = useQuery(databaseEnginesQuery)
  const projects = useQuery(projectsQuery)
  const { data: me } = useQuery(meQuery)
  const canEdit = me ? hasRole(me.role, 'developer') : false
  const canDelete = me ? hasRole(me.role, 'admin') : false

  const [creating, setCreating] = useState(false)
  const [connecting, setConnecting] = useState<Database | null>(null)
  const [deleting, setDeleting] = useState<Database | null>(null)
  const [backingUp, setBackingUp] = useState<Database | null>(null)

  const queryClient = useQueryClient()
  const refresh = () => queryClient.invalidateQueries({ queryKey: databasesQuery.queryKey })
  const start = useMutation({
    mutationFn: (id: string) => unwrap(api.POST('/api/v1/databases/{id}/start', { params: { path: { id } } })),
    meta: { success: 'Database starting' },
    onSettled: refresh,
  })
  const stop = useMutation({
    mutationFn: (id: string) => unwrap(api.POST('/api/v1/databases/{id}/stop', { params: { path: { id } } })),
    meta: { success: 'Database stopped' },
    onSettled: refresh,
  })

  const engineLabel = (engine: Engine) => engines.data?.find((e) => e.engine === engine)?.label ?? engine
  const hasProjects = (projects.data?.length ?? 0) > 0
  const newButton = canEdit ? (
    <Button onClick={() => setCreating(true)} disabled={!hasProjects}>
      <Plus data-icon="inline-start" />
      New database
    </Button>
  ) : undefined

  return (
    <Page
      title="Databases"
      actions={databases.data?.length ? newButton : undefined}
    >
      {databases.isPending ? (
        <Panel>
          <div className="space-y-2">
            {Array.from({ length: 3 }, (_, i) => (
              <Skeleton key={i} className="h-9" />
            ))}
          </div>
        </Panel>
      ) : databases.isError ? (
        <ErrorState error={databases.error} onRetry={() => void databases.refetch()} />
      ) : databases.data.length === 0 ? (
        <EmptyState
          icon={DatabaseIcon}
          title="No databases"
          action={
            hasProjects || !canEdit ? (
              newButton
            ) : (
              <Button nativeButton={false} render={<Link to="/projects" />}>
                Create a project first
              </Button>
            )
          }
        >
          Private PostgreSQL, MySQL, MariaDB, Redis, or MongoDB instances reachable only inside their project.
        </EmptyState>
      ) : (
        <Panel flush>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="pl-4">Name</TableHead>
                <TableHead>Status</TableHead>
                <TableHead className="hidden md:table-cell">Project</TableHead>
                <TableHead className="hidden sm:table-cell">Internal address</TableHead>
                <TableHead className="hidden lg:table-cell">Created</TableHead>
                <TableHead className="w-10 pr-4" />
              </TableRow>
            </TableHeader>
            <TableBody>
              {databases.data.map((db, index) => {
                const look = DATABASE_STATUS[db.status]
                return (
                  <TableRow key={db.id} index={index}>
                    <TableCell className="pl-4">
                      <div className="flex items-center gap-2.5">
                        <span className="grid size-7 shrink-0 place-items-center rounded-md bg-muted text-muted-foreground">
                          <EngineMark engine={db.engine} className="size-4" />
                        </span>
                        <div className="min-w-0">
                          <span className="block max-w-52 truncate font-medium text-foreground">{db.name}</span>
                          <span className="block text-[12px] text-muted-foreground">
                            {engineLabel(db.engine)} {db.version}
                          </span>
                        </div>
                      </div>
                    </TableCell>
                    <TableCell className="max-w-72">
                      <Status tone={look.tone}>{look.label}</Status>
                      {db.error ? (
                        <p className="mt-1 line-clamp-2 text-[12px] whitespace-normal text-fail" title={db.error}>
                          {db.error}
                        </p>
                      ) : null}
                    </TableCell>
                    <TableCell className="hidden text-muted-foreground md:table-cell">{db.project_name}</TableCell>
                    <TableCell className="hidden sm:table-cell">
                      <span className="inline-flex items-center gap-1">
                        <code className="text-[12.5px] text-foreground">
                          {db.host}:{db.port}
                        </code>
                        <CopyButton value={`${db.host}:${db.port}`} label="Copy address" />
                      </span>
                    </TableCell>
                    <TableCell className="hidden text-muted-foreground lg:table-cell">{timeAgo(db.created_at)}</TableCell>
                    <TableCell className="pr-4 text-right">
                      {canEdit ? (
                        <DropdownMenu>
                          <DropdownTrigger
                            render={<Button variant="ghost" size="icon-sm" aria-label={`Actions for ${db.name}`} />}
                          >
                            <Ellipsis />
                          </DropdownTrigger>
                          <DropdownContent align="end" className="w-48">
                            <MenuItem
                              index={0}
                              icon={KeyRound}
                              label="Connection details"
                              onSelect={() => setConnecting(db)}
                            />
                            <MenuItem
                              index={1}
                              icon={Archive}
                              label="Backups"
                              onSelect={() => setBackingUp(db)}
                            />
                            {db.status === 'running' ? (
                              <MenuItem
                                index={2}
                                icon={Square}
                                label="Stop"
                                disabled={stop.isPending}
                                onSelect={() => stop.mutate(db.id)}
                              />
                            ) : (
                              <MenuItem
                                index={2}
                                icon={Play}
                                label="Start"
                                disabled={db.status === 'creating' || start.isPending}
                                onSelect={() => start.mutate(db.id)}
                              />
                            )}
                            {canDelete ? (
                              <>
                                <DropdownSeparator />
                                <MenuItem
                                  index={3}
                                  icon={Trash2}
                                  label="Delete database"
                                  destructive
                                  disabled={db.status === 'creating'}
                                  onSelect={() => setDeleting(db)}
                                />
                              </>
                            ) : null}
                          </DropdownContent>
                        </DropdownMenu>
                      ) : null}
                    </TableCell>
                  </TableRow>
                )
              })}
            </TableBody>
          </Table>
        </Panel>
      )}

      <NewDatabaseDialog open={creating} onOpenChange={setCreating} />
      <ConnectionDialog database={connecting} onClose={() => setConnecting(null)} />
      <DeleteDialog database={deleting} onClose={() => setDeleting(null)} />
      <BackupsSheet
        database={backingUp && (databases.data?.find((d) => d.id === backingUp.id) ?? backingUp)}
        canBackup={canEdit}
        canManage={canDelete}
        onClose={() => setBackingUp(null)}
      />
    </Page>
  )
}

export function ConnectionDialog({ database, onClose }: { database: Database | null; onClose: () => void }) {
  return (
    <Dialog open={database !== null} onOpenChange={(open) => (open ? undefined : onClose())}>
      <DialogContent size="lg">
        {database ? <ConnectionDetails key={database.id} database={database} onClose={onClose} /> : null}
      </DialogContent>
    </Dialog>
  )
}

function ConnectionDetails({ database: db, onClose }: { database: Database; onClose: () => void }) {
  const [shown, setShown] = useState(false)
  const reveal = useMutation({
    mutationFn: () => unwrap(api.POST('/api/v1/databases/{id}/reveal', { params: { path: { id: db.id } } })),
    onSuccess: () => setShown(true),
  })
  const secret = shown ? reveal.data : undefined
  const masked = '••••••••••••'

  const rows: { label: string; value: string; copy?: string }[] = [
    { label: 'Host', value: db.host },
    { label: 'Port', value: String(db.port) },
    { label: 'User', value: db.username },
    { label: 'Database', value: db.db_name },
    { label: 'Password', value: secret?.password ?? masked, copy: secret?.password },
    {
      label: 'URL',
      value: secret?.url ?? `${db.engine}://${db.username}:${masked}@${db.host}:${db.port}/${db.db_name}`,
      copy: secret?.url,
    },
  ]

  return (
    <>
      <DialogHeader>
        <DialogTitle>Connect to {db.name}</DialogTitle>
        <DialogDescription>
          Private to apps in {db.project_name}. Store the URL as a secret env var (e.g. <code className="font-mono">DATABASE_URL</code>).
        </DialogDescription>
      </DialogHeader>
      <dl className="divide-y divide-border/80 rounded-lg border border-border/80 text-[13px]">
        {rows.map((row) => (
          <div key={row.label} className="flex min-h-10 items-center gap-3 py-1 pr-1 pl-3">
            <dt className="w-20 shrink-0 text-muted-foreground">{row.label}</dt>
            <dd className="min-w-0 flex-1 overflow-x-auto whitespace-nowrap">
              <code className="text-[12.5px]">{row.value}</code>
            </dd>
            {row.copy !== undefined || !['Password', 'URL'].includes(row.label) ? (
              <CopyButton value={row.copy ?? row.value} label={`Copy ${row.label.toLowerCase()}`} />
            ) : null}
          </div>
        ))}
      </dl>
      {db.host_port ? (
        <p className="text-[12.5px] text-muted-foreground">
          Localhost port <code className="font-mono">127.0.0.1:{db.host_port}</code> (use an SSH tunnel from your machine).
        </p>
      ) : null}
      <DialogFooter>
        <Button
          variant="outline"
          disabled={reveal.isPending}
          onClick={() => (shown ? setShown(false) : reveal.mutate())}
        >
          {reveal.isPending ? <Spinner data-icon="inline-start" /> : shown ? <EyeOff data-icon="inline-start" /> : <Eye data-icon="inline-start" />}
          {shown ? 'Hide password' : 'Show password'}
        </Button>
        <Button onClick={onClose}>Done</Button>
      </DialogFooter>
    </>
  )
}

function DeleteDialog({ database, onClose }: { database: Database | null; onClose: () => void }) {
  const queryClient = useQueryClient()
  const [typed, setTyped] = useState('')
  const close = () => {
    setTyped('')
    onClose()
  }
  const remove = useMutation({
    mutationFn: (id: string) => unwrap(api.DELETE('/api/v1/databases/{id}', { params: { path: { id } } })),
    meta: { success: 'Database deleted' },
    onSuccess: async () => {
      close()
      await queryClient.invalidateQueries({ queryKey: databasesQuery.queryKey })
    },
  })

  return (
    <AlertDialog open={database !== null} onOpenChange={(open) => (open ? undefined : close())}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Delete “{database?.name}”?</AlertDialogTitle>
          <AlertDialogDescription>
            The database, its data, and all backups will be permanently erased. This cannot be undone.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <Field>
          <FieldLabel htmlFor="db-confirm">Type {database?.name} to confirm</FieldLabel>
          <Input id="db-confirm" value={typed} onChange={(e) => setTyped(e.target.value)} autoComplete="off" />
        </Field>
        <AlertDialogFooter>
          <AlertDialogCancel>Keep database</AlertDialogCancel>
          <AlertDialogAction
            variant="destructive"
            disabled={remove.isPending || typed !== database?.name}
            onClick={() => database && remove.mutate(database.id)}
          >
            Delete database
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
