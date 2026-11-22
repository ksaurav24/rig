import { useQuery } from '@tanstack/react-query'
import { Link, createFileRoute, notFound, useNavigate } from '@tanstack/react-router'
import {
  AppWindow,
  Archive,
  ChevronDown,
  ChevronRight,
  Database,
  Ellipsis,
  GitBranch,
  KeyRound,
  LayoutGrid,
  Package,
  Pencil,
  Pin,
  PinOff,
  Plus,
  Trash2,
} from 'lucide-react'
import { useMemo, useState } from 'react'
import type { Schemas } from '@/api/client'
import { appsQuery, databasesQuery, meQuery, projectsQuery } from '@/api/queries'
import { BackupsSheet } from '@/components/app/backups-sheet'
import { CopyButton } from '@/components/app/copy-button'
import { NewAppDialog } from '@/components/app/new-app-dialog'
import { NewDatabaseDialog } from '@/components/app/new-database-dialog'
import { Page, Panel } from '@/components/app/page'
import { DeleteProjectDialog, EditProjectDialog } from '@/components/app/project-dialogs'
import { EngineMark } from '@/components/app/service-icons'
import { EmptyState, ErrorState, Status } from '@/components/app/state'
import {
  DropdownContent,
  DropdownMenu,
  DropdownSeparator,
  DropdownTrigger,
  MenuItem,
} from '@/components/fluid/dropdown'
import { Tooltip } from '@/components/fluid/tooltip'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { timeAgo } from '@/lib/format'
import { hasRole } from '@/lib/nav'
import { usePreferences } from '@/lib/preferences'
import { APP_STATUS, DATABASE_STATUS } from '@/lib/status'
import { cn } from '@/lib/utils'
import { ConnectionDialog } from '@/routes/_app/databases/index'

type App = Schemas['App']
type Db = Schemas['Database']

export const Route = createFileRoute('/_app/projects/$projectId')({
  loader: async ({ context, params }) => {
    void context.queryClient.prefetchQuery(appsQuery)
    void context.queryClient.prefetchQuery(databasesQuery)
    const projects = await context.queryClient.ensureQueryData(projectsQuery)
    const project = projects.find((p) => p.id === params.projectId)
    if (!project) throw notFound()
    return { crumb: project.name, parents: [{ label: 'Projects', to: '/projects' }] }
  },
  notFoundComponent: () => (
    <Page title="Project not found">
      <EmptyState
        icon={LayoutGrid}
        title="This project doesn’t exist"
        action={
          <Button nativeButton={false} render={<Link to="/projects" />} variant="outline">
            All projects
          </Button>
        }
      >
        It may have been deleted, or the link is wrong.
      </EmptyState>
    </Page>
  ),
  errorComponent: ({ error, reset }) => (
    <div className="mx-auto w-full max-w-6xl px-4 py-6 md:px-8 md:py-8">
      <ErrorState error={error} onRetry={reset} />
    </div>
  ),
  component: ProjectPage,
})

function ProjectPage() {
  const { projectId } = Route.useParams()
  const navigate = useNavigate()
  const projects = useQuery(projectsQuery)
  const apps = useQuery(appsQuery)
  const databases = useQuery(databasesQuery)
  const { data: me } = useQuery(meQuery)
  const { isProjectPinned, togglePinnedProject } = usePreferences()
  const canEdit = me ? hasRole(me.role, 'developer') : false
  const canDelete = me ? hasRole(me.role, 'admin') : false

  const [addingApp, setAddingApp] = useState(false)
  const [addingDb, setAddingDb] = useState(false)
  const [deleting, setDeleting] = useState(false)
  const [editing, setEditing] = useState(false)
  const [connectingDb, setConnectingDb] = useState<Db | null>(null)
  const [backingUpDb, setBackingUpDb] = useState<Db | null>(null)

  const project = projects.data?.find((p) => p.id === projectId) ?? null
  const projectApps = useMemo(() => (apps.data ?? []).filter((a) => a.project_id === projectId), [apps.data, projectId])
  const projectDbs = useMemo(
    () => (databases.data ?? []).filter((d) => d.project_id === projectId),
    [databases.data, projectId],
  )

  if (!project) {
    if (projects.isPending) return <Skeleton className="m-8 h-40" />
    return (
      <Page title="Project not found">
        <EmptyState
          icon={Package}
          title="This project is gone"
          action={
            <Button nativeButton={false} render={<Link to="/projects" />}>
              All projects
            </Button>
          }
        >
          It was deleted, or it belongs to another organization.
        </EmptyState>
      </Page>
    )
  }

  const create = canEdit ? (
    <DropdownMenu>
      <DropdownTrigger render={<Button />}>
        <Plus data-icon="inline-start" />
        Create service
        <ChevronDown data-icon="inline-end" className="opacity-70" />
      </DropdownTrigger>
      <DropdownContent align="end" className="w-56">
        <MenuItem index={0} icon={AppWindow} label="App" onSelect={() => setAddingApp(true)} />
        <MenuItem index={1} icon={Database} label="Database" onSelect={() => setAddingDb(true)} />
        <DropdownSeparator />
        <MenuItem
          index={2}
          icon={LayoutGrid}
          label="From template"
          onSelect={() => void navigate({ to: '/catalog', search: { project: project.id } })}
        />
      </DropdownContent>
    </DropdownMenu>
  ) : null

  const pending = apps.isPending || databases.isPending
  const error = apps.error ?? databases.error
  const empty = !pending && !error && projectApps.length === 0 && projectDbs.length === 0

  return (
    <Page
      title={project.name}
      description={project.description || undefined}
      actions={
        <div className="flex items-center gap-1.5">
          <Tooltip content={isProjectPinned(project.id) ? 'Unpin from sidebar' : 'Pin to sidebar'}>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={isProjectPinned(project.id) ? 'Unpin project from sidebar' : 'Pin project to sidebar'}
              aria-pressed={isProjectPinned(project.id)}
              onClick={() => togglePinnedProject(project.id)}
              className={isProjectPinned(project.id) ? 'text-primary' : undefined}
            >
              {isProjectPinned(project.id) ? <PinOff /> : <Pin />}
            </Button>
          </Tooltip>
          {canEdit ? (
            <DropdownMenu>
              <DropdownTrigger render={<Button variant="ghost" size="icon-sm" aria-label="Project actions" />}>
                <Ellipsis />
              </DropdownTrigger>
              <DropdownContent align="end" className="w-48">
                <MenuItem index={0} icon={Pencil} label="Edit project" onSelect={() => setEditing(true)} />
                {canDelete ? (
                  <>
                    <DropdownSeparator />
                    <MenuItem
                      index={1}
                      icon={Trash2}
                      label="Delete project"
                      destructive
                      onSelect={() => setDeleting(true)}
                    />
                  </>
                ) : null}
              </DropdownContent>
            </DropdownMenu>
          ) : null}
          {empty ? null : create}
        </div>
      }
    >
      {pending ? (
        <Panel flush>
          {Array.from({ length: 3 }, (_, i) => (
            <div key={i} className="flex items-center gap-3 border-b border-border/80 px-4 py-3 last:border-0">
              <Skeleton className="size-8 rounded-md" />
              <Skeleton className="h-4 w-48" />
            </div>
          ))}
        </Panel>
      ) : error ? (
        <ErrorState
          error={error}
          onRetry={() => {
            void apps.refetch()
            void databases.refetch()
          }}
        />
      ) : empty ? (
        <FirstService
          canEdit={canEdit}
          onApp={() => setAddingApp(true)}
          onDb={() => setAddingDb(true)}
          projectId={project.id}
        />
      ) : (
        <div className="grid gap-6">
          <Panel flush title="Apps">
            {projectApps.length ? (
              <ul className="divide-y divide-border/80">
                {projectApps.map((app) => (
                  <AppRow key={app.id} app={app} />
                ))}
              </ul>
            ) : canEdit ? (
              <InlineAdd onClick={() => setAddingApp(true)}>Add an app</InlineAdd>
            ) : (
              <p className="px-4 py-3 text-[13px] text-muted-foreground">No apps yet.</p>
            )}
          </Panel>
          <Panel flush title="Databases">
            {projectDbs.length ? (
              <ul className="divide-y divide-border/80">
                {projectDbs.map((db) => (
                  <DbRow
                    key={db.id}
                    db={db}
                    canEdit={canEdit}
                    onConnect={() => setConnectingDb(db)}
                    onBackups={() => setBackingUpDb(db)}
                  />
                ))}
              </ul>
            ) : canEdit ? (
              <InlineAdd onClick={() => setAddingDb(true)}>Add a database</InlineAdd>
            ) : (
              <p className="px-4 py-3 text-[13px] text-muted-foreground">No databases yet.</p>
            )}
          </Panel>
        </div>
      )}

      <NewAppDialog project={addingApp ? project : null} onClose={() => setAddingApp(false)} />
      <NewDatabaseDialog open={addingDb} onOpenChange={setAddingDb} projectId={project.id} />
      <EditProjectDialog project={editing ? project : null} onClose={() => setEditing(false)} />
      <DeleteProjectDialog
        project={deleting ? project : null}
        databases={projectDbs}
        projects={projects.data ?? []}
        onClose={() => setDeleting(false)}
        onDeleted={() => void navigate({ to: '/projects' })}
      />
      <ConnectionDialog database={connectingDb} onClose={() => setConnectingDb(null)} />
      <BackupsSheet
        database={backingUpDb && (databases.data?.find((d) => d.id === backingUpDb.id) ?? backingUpDb)}
        canBackup={canEdit}
        canManage={canDelete}
        onClose={() => setBackingUpDb(null)}
      />
    </Page>
  )
}

function InlineAdd({ onClick, children }: { onClick: () => void; children: string }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex w-full items-center gap-2 px-4 py-3 text-left text-[13px] text-muted-foreground transition-colors outline-none hover:bg-hover hover:text-foreground focus-visible:bg-hover"
    >
      <Plus className="size-3.5" aria-hidden="true" />
      {children}
    </button>
  )
}

/** The empty project: the three ways in, laid out as the next step. */
function FirstService({
  canEdit,
  onApp,
  onDb,
  projectId,
}: {
  canEdit: boolean
  onApp: () => void
  onDb: () => void
  projectId: string
}) {
  if (!canEdit) {
    return (
      <EmptyState icon={LayoutGrid} title="Nothing in this project yet">
        A developer or admin can add apps and databases here.
      </EmptyState>
    )
  }
  const tile =
    'flex flex-col items-start gap-3 rounded-xl bg-card p-5 text-left outline-none shadow-[var(--shadow-2)] transition-[box-shadow,background-color] duration-150 hover:bg-hover hover:shadow-[0_0_0_1px_var(--ring)] focus-visible:ring-1 focus-visible:ring-[color:var(--focus-ring)]'
  return (
    <div className="grid gap-4">
      <div>
        <h2 className="text-[15px] font-medium">Add your first service</h2>
        <p className="text-[13px] text-muted-foreground">Apps and databases here share a private network.</p>
      </div>
      <div className="grid gap-3 sm:grid-cols-3">
        <button type="button" onClick={onApp} className={tile}>
          <AppWindow className="size-5 text-muted-foreground" aria-hidden="true" />
          <span>
            <span className="block font-medium text-foreground">App</span>
            <span className="mt-0.5 block text-[12.5px] text-muted-foreground">GitHub repo, Git URL, or Docker image.</span>
          </span>
        </button>
        <button type="button" onClick={onDb} className={tile}>
          <Database className="size-5 text-muted-foreground" aria-hidden="true" />
          <span>
            <span className="block font-medium text-foreground">Database</span>
            <span className="mt-0.5 block text-[12.5px] text-muted-foreground">Postgres, MySQL, MariaDB, Redis, or MongoDB.</span>
          </span>
        </button>
        <Link to="/catalog" search={{ project: projectId }} className={tile}>
          <LayoutGrid className="size-5 text-muted-foreground" aria-hidden="true" />
          <span>
            <span className="block font-medium text-foreground">Template</span>
            <span className="mt-0.5 block text-[12.5px] text-muted-foreground">One-click self-hosted apps from the catalog.</span>
          </span>
        </Link>
      </div>
    </div>
  )
}

function ServiceIcon({ children, className }: { children: React.ReactNode; className?: string }) {
  return (
    <span className={cn('grid size-8 shrink-0 place-items-center rounded-lg bg-muted text-muted-foreground', className)}>
      {children}
    </span>
  )
}

function AppRow({ app }: { app: App }) {
  const look = APP_STATUS[app.status]
  const Source = app.source_kind === 'git' ? GitBranch : Package
  const source =
    app.source_kind === 'git'
      ? `${app.git_url?.replace(/^https?:\/\/(www\.)?(github\.com\/)?/, '').replace(/\.git$/, '')}${app.git_branch ? ` · ${app.git_branch}` : ''}`
      : app.image
  return (
    <li>
      <Link
        to="/apps/$appId"
        params={{ appId: app.id }}
        className="group flex items-center gap-3 px-4 py-3 transition-colors outline-none hover:bg-hover focus-visible:bg-hover"
      >
        <ServiceIcon>
          <AppWindow className="size-4" aria-hidden="true" />
        </ServiceIcon>
        <div className="min-w-0 flex-1">
          <div className="truncate font-medium text-foreground">{app.name}</div>
          <div className="flex items-center gap-1.5 text-[12px] text-muted-foreground">
            <Source className="size-3.5 shrink-0" aria-hidden="true" />
            <span className="truncate font-mono">{source}</span>
          </div>
        </div>
        <span className="hidden max-w-56 truncate font-mono text-[12px] text-muted-foreground md:block">
          {app.domains[0] ?? ''}
          {app.domains.length > 1 ? ` +${app.domains.length - 1}` : ''}
        </span>
        <span className="hidden w-24 text-right text-[12.5px] text-muted-foreground lg:block">{timeAgo(app.updated_at)}</span>
        <Status tone={look.tone} className="w-24 justify-end">
          {look.label}
        </Status>
        <ChevronRight className="size-4 shrink-0 text-muted-foreground/60 transition-transform group-hover:translate-x-0.5" aria-hidden="true" />
      </Link>
    </li>
  )
}

function DbRow({
  db,
  canEdit,
  onConnect,
  onBackups,
}: {
  db: Db
  canEdit: boolean
  onConnect: () => void
  onBackups: () => void
}) {
  const look = DATABASE_STATUS[db.status]
  const address = `${db.host}:${db.port}`
  return (
    <li className="flex items-center gap-3 px-4 py-3">
      <ServiceIcon>
        <EngineMark engine={db.engine} className="size-4" />
      </ServiceIcon>
      <div className="min-w-0 flex-1">
        <div className="truncate font-medium text-foreground">{db.name}</div>
        <div className="flex items-center gap-1 text-[12px] text-muted-foreground">
          <span className="truncate font-mono">{address}</span>
          <CopyButton value={address} label="Copy address" />
        </div>
      </div>
      <span className="hidden text-[12.5px] text-muted-foreground md:block">
        {db.engine} {db.version}
      </span>
      <Status tone={look.tone} className="w-24 justify-end">
        {look.label}
      </Status>
      {canEdit ? (
        <div className="flex shrink-0 items-center gap-1">
          <Tooltip content="Connection details">
            <Button variant="ghost" size="icon-sm" aria-label={`Connection details for ${db.name}`} onClick={onConnect}>
              <KeyRound />
            </Button>
          </Tooltip>
          <Tooltip content="Backups">
            <Button variant="ghost" size="icon-sm" aria-label={`Backups for ${db.name}`} onClick={onBackups}>
              <Archive />
            </Button>
          </Tooltip>
        </div>
      ) : (
        <Link
          to="/databases"
          className="text-[12.5px] text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
          aria-label={`Manage ${db.name}`}
        >
          Manage
        </Link>
      )}
    </li>
  )
}
