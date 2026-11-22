import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Link, createFileRoute } from '@tanstack/react-router'
import { Archive, Database as DatabaseIcon, Ellipsis, Eye, EyeOff, KeyRound, Play, Plus, Square, Trash2 } from 'lucide-react'
import { useState } from 'react'
import { api, unwrap, type Schemas } from '@/api/client'
import { databaseEnginesQuery, databasesQuery, meQuery, projectsQuery } from '@/api/queries'
import { BackupsSheet } from '@/components/app/backups-sheet'
import { NewDatabaseDialog } from '@/components/app/new-database-dialog'
import { CopyButton } from '@/components/app/copy-button'
import { Page, Panel } from '@/components/app/page'
import { EmptyState, ErrorState, Status } from '@/components/app/state'
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
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Field, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
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
      description="Each database runs in its own container with its own volume. Apps in the same project reach it by name; nothing is exposed to the internet."
      actions={databases.data?.length ? newButton : undefined}
    >
      {databases.isPending ? (
        <Panel>
          <div className="space-y-2">
            {Array.from({ length: 3 }, (_, i) => (
              <Skeleton key={i} className="h-8" />
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
          PostgreSQL, MySQL, MariaDB, Redis or MongoDB, started in about a minute and reachable from the apps in its project.
        </EmptyState>
      ) : (
        <Panel flush>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="pl-4">Name</TableHead>
                <TableHead>Status</TableHead>
                <TableHead className="hidden md:table-cell">Project</TableHead>
                <TableHead className="hidden sm:table-cell">Address inside the project</TableHead>
                <TableHead className="hidden lg:table-cell">Created</TableHead>
                <TableHead className="w-10 pr-4" />
              </TableRow>
            </TableHeader>
            <TableBody>
              {databases.data.map((db) => {
                const look = DATABASE_STATUS[db.status]
                return (
                  <TableRow key={db.id}>
                    <TableCell className="pl-4">
                      <span className="block max-w-56 truncate font-medium">{db.name}</span>
                      <span className="text-[0.8125rem] text-muted-foreground">
                        {engineLabel(db.engine)} {db.version}
                      </span>
                    </TableCell>
                    <TableCell className="max-w-72">
                      <Status tone={look.tone}>{look.label}</Status>
                      {db.error ? (
                        <p className="mt-1 line-clamp-2 text-[0.8125rem] whitespace-normal text-muted-foreground" title={db.error}>
                          {db.error}
                        </p>
                      ) : null}
                    </TableCell>
                    <TableCell className="hidden text-muted-foreground md:table-cell">{db.project_name}</TableCell>
                    <TableCell className="hidden sm:table-cell">
                      <span className="inline-flex items-center gap-1">
                        <code className="text-[0.8125rem]">
                          {db.host}:{db.port}
                        </code>
                        <CopyButton value={`${db.host}:${db.port}`} label="Copy address" />
                      </span>
                    </TableCell>
                    <TableCell className="hidden text-muted-foreground lg:table-cell">{timeAgo(db.created_at)}</TableCell>
                    <TableCell className="pr-4 text-right">
                      {canEdit ? (
                        <DropdownMenu>
                          <DropdownMenuTrigger
                            render={<Button variant="ghost" size="icon-sm" aria-label={`Actions for ${db.name}`} />}
                          >
                            <Ellipsis />
                          </DropdownMenuTrigger>
                          <DropdownMenuContent align="end" className="min-w-48">
                            <DropdownMenuItem onClick={() => setConnecting(db)}>
                              <KeyRound />
                              Connection details
                            </DropdownMenuItem>
                            <DropdownMenuItem onClick={() => setBackingUp(db)}>
                              <Archive />
                              Backups
                            </DropdownMenuItem>
                            {db.status === 'running' ? (
                              <DropdownMenuItem disabled={stop.isPending} onClick={() => stop.mutate(db.id)}>
                                <Square />
                                Stop
                              </DropdownMenuItem>
                            ) : (
                              <DropdownMenuItem
                                disabled={db.status === 'creating' || start.isPending}
                                onClick={() => start.mutate(db.id)}
                              >
                                <Play />
                                Start
                              </DropdownMenuItem>
                            )}
                            {canDelete ? (
                              <>
                                <DropdownMenuSeparator />
                                <DropdownMenuItem
                                  variant="destructive"
                                  disabled={db.status === 'creating'}
                                  onClick={() => setDeleting(db)}
                                >
                                  <Trash2 />
                                  Delete database
                                </DropdownMenuItem>
                              </>
                            ) : null}
                          </DropdownMenuContent>
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

function ConnectionDialog({ database, onClose }: { database: Database | null; onClose: () => void }) {
  return (
    <Dialog open={database !== null} onOpenChange={(open) => (open ? undefined : onClose())}>
      <DialogContent className="sm:max-w-xl">
        {/* Keyed so revealed credentials never carry over to another database. */}
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
          These work from apps in the {db.project_name} project. Add the URL to an app as a secret variable, for example
          DATABASE_URL.
        </DialogDescription>
      </DialogHeader>
      <dl className="divide-y rounded-md border text-[0.8125rem]">
        {rows.map((row) => (
          <div key={row.label} className="flex min-h-10 items-center gap-3 py-1 pr-1 pl-3">
            <dt className="w-20 shrink-0 text-muted-foreground">{row.label}</dt>
            <dd className="min-w-0 flex-1 overflow-x-auto whitespace-nowrap">
              <code>{row.value}</code>
            </dd>
            {row.copy !== undefined || !['Password', 'URL'].includes(row.label) ? (
              <CopyButton value={row.copy ?? row.value} label={`Copy ${row.label.toLowerCase()}`} />
            ) : null}
          </div>
        ))}
      </dl>
      {db.host_port ? (
        <p className="text-[0.8125rem] text-muted-foreground">
          On the server itself it also listens on <code>127.0.0.1:{db.host_port}</code>, which changes each time the
          database starts. Use an SSH tunnel to reach it from your machine.
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
            The database, every row in it and all of its backups are erased from the server. Download a backup first if you might need it. This cannot be undone.
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
