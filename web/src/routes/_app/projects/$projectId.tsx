import { useQuery } from '@tanstack/react-query'
import { Link, createFileRoute, notFound, useNavigate } from '@tanstack/react-router'
import { AppWindow, ChevronDown, ChevronRight, Database, Ellipsis, GitBranch, LayoutGrid, Package, Pencil, Plus, Trash2 } from 'lucide-react'
import { useMemo, useState } from 'react'
import type { Schemas } from '@/api/client'
import { appsQuery, databasesQuery, meQuery, projectsQuery } from '@/api/queries'
import { CopyButton } from '@/components/app/copy-button'
import { NewAppDialog } from '@/components/app/new-app-dialog'
import { NewDatabaseDialog } from '@/components/app/new-database-dialog'
import { Page, Panel } from '@/components/app/page'
import { DeleteProjectDialog, EditProjectDialog } from '@/components/app/project-dialogs'
import { EngineMark } from '@/components/app/service-icons'
import { EmptyState, ErrorState, Status } from '@/components/app/state'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Skeleton } from '@/components/ui/skeleton'
import { timeAgo } from '@/lib/format'
import { hasRole } from '@/lib/nav'
import { APP_STATUS, DATABASE_STATUS } from '@/lib/status'
import { cn } from '@/lib/utils'

type App = Schemas['App']
type Db = Schemas['Database']

export const Route = createFileRoute('/_app/projects/$projectId')({
  loader: async ({ context, params }) => {
    void context.queryClient.prefetchQuery(appsQuery)
    void context.queryClient.prefetchQuery(databasesQuery)
    // There is no single-project endpoint; the list is small and cached.
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
  const canEdit = me ? hasRole(me.role, 'developer') : false
  const canDelete = me ? hasRole(me.role, 'admin') : false

  const [addingApp, setAddingApp] = useState(false)
  const [addingDb, setAddingDb] = useState(false)
  const [deleting, setDeleting] = useState(false)
  const [editing, setEditing] = useState(false)

  const project = projects.data?.find((p) => p.id === projectId) ?? null
  const projectApps = useMemo(() => (apps.data ?? []).filter((a) => a.project_id === projectId), [apps.data, projectId])
  const projectDbs = useMemo(
    () => (databases.data ?? []).filter((d) => d.project_id === projectId),
    [databases.data, projectId],
  )

  // Deleted elsewhere while open: the cached list no longer has it.
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
      <DropdownMenuTrigger render={<Button />}>
        <Plus data-icon="inline-start" />
        Create service
        <ChevronDown data-icon="inline-end" className="opacity-60" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-64">
        <MenuEntry icon={AppWindow} title="App" hint="From GitHub, a Git URL or an image" onClick={() => setAddingApp(true)} />
        <MenuEntry icon={Database} title="Database" hint="Postgres, MySQL, Redis and more" onClick={() => setAddingDb(true)} />
        <DropdownMenuSeparator />
        <MenuEntry
          icon={LayoutGrid}
          title="From template"
          hint="One-click apps from the catalog"
          onClick={() => void navigate({ to: '/catalog', search: { project: project.id } })}
        />
      </DropdownMenuContent>
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
          {canEdit ? (
            <DropdownMenu>
              <DropdownMenuTrigger render={<Button variant="ghost" size="icon-sm" aria-label="Project actions" />}>
                <Ellipsis />
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem onClick={() => setEditing(true)}>
                  <Pencil />
                  Rename or describe
                </DropdownMenuItem>
                {canDelete ? (
                  <>
                    <DropdownMenuSeparator />
                    <DropdownMenuItem variant="destructive" onClick={() => setDeleting(true)}>
                      <Trash2 />
                      Delete project
                    </DropdownMenuItem>
                  </>
                ) : null}
              </DropdownMenuContent>
            </DropdownMenu>
          ) : null}
          {empty ? null : create}
        </div>
      }
    >
      {pending ? (
        <Panel flush>
          {Array.from({ length: 3 }, (_, i) => (
            <div key={i} className="flex items-center gap-3 border-b px-4 py-3 last:border-0">
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
          <Panel flush title="Apps" description={projectApps.length ? undefined : 'No apps yet.'}>
            {projectApps.length ? (
              <ul className="divide-y">
                {projectApps.map((app) => (
                  <AppRow key={app.id} app={app} />
                ))}
              </ul>
            ) : canEdit ? (
              <InlineAdd onClick={() => setAddingApp(true)}>Add an app</InlineAdd>
            ) : null}
          </Panel>
          <Panel
            flush
            title="Databases"
            description={projectDbs.length ? 'Reachable from this project’s apps by the host name shown.' : 'No databases yet.'}
          >
            {projectDbs.length ? (
              <ul className="divide-y">
                {projectDbs.map((db) => (
                  <DbRow key={db.id} db={db} />
                ))}
              </ul>
            ) : canEdit ? (
              <InlineAdd onClick={() => setAddingDb(true)}>Add a database</InlineAdd>
            ) : null}
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
    </Page>
  )
}

function MenuEntry({
  icon: Icon,
  title,
  hint,
  onClick,
}: {
  icon: typeof AppWindow
  title: string
  hint: string
  onClick: () => void
}) {
  return (
    <DropdownMenuItem onClick={onClick} className="items-start gap-2.5 py-2">
      <Icon className="mt-0.5" />
      <span>
        <span className="block font-medium">{title}</span>
        <span className="block text-[12px] text-muted-foreground">{hint}</span>
      </span>
    </DropdownMenuItem>
  )
}

function InlineAdd({ onClick, children }: { onClick: () => void; children: string }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex w-full items-center gap-2 px-4 py-3 text-left text-[13px] text-muted-foreground outline-none hover:bg-hover hover:text-foreground focus-visible:bg-hover"
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
    'flex flex-col items-start gap-3 rounded-lg bg-card p-5 text-left outline-none shadow-[0_0_0_1px_var(--border)] transition-[box-shadow] duration-150 hover:shadow-[0_0_0_1px_var(--ring)] focus-visible:ring-1 focus-visible:ring-[color:var(--focus-ring)]'
  return (
    <div className="grid gap-4">
      <div>
        <h2 className="font-medium">Add the first service</h2>
        <p className="text-[13px] text-muted-foreground">Everything in this project shares a private network.</p>
      </div>
      <div className="grid gap-3 sm:grid-cols-3">
        <button type="button" onClick={onApp} className={tile}>
          <AppWindow className="size-5 text-muted-foreground" aria-hidden="true" />
          <span>
            <span className="block font-medium">App</span>
            <span className="block text-[13px] text-muted-foreground">Deploy from GitHub, a Git URL or a Docker image.</span>
          </span>
        </button>
        <button type="button" onClick={onDb} className={tile}>
          <Database className="size-5 text-muted-foreground" aria-hidden="true" />
          <span>
            <span className="block font-medium">Database</span>
            <span className="block text-[13px] text-muted-foreground">Postgres, MySQL, MariaDB, Redis or MongoDB with backups.</span>
          </span>
        </button>
        <Link to="/catalog" search={{ project: projectId }} className={tile}>
          <LayoutGrid className="size-5 text-muted-foreground" aria-hidden="true" />
          <span>
            <span className="block font-medium">Template</span>
            <span className="block text-[13px] text-muted-foreground">Ready-made apps like n8n or Uptime Kuma.</span>
          </span>
        </Link>
      </div>
    </div>
  )
}

function ServiceIcon({ children, className }: { children: React.ReactNode; className?: string }) {
  return (
    <span className={cn('grid size-8 shrink-0 place-items-center rounded-md bg-muted text-muted-foreground', className)}>
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
        className="group flex items-center gap-3 px-4 py-3 outline-none hover:bg-hover focus-visible:bg-hover"
      >
        <ServiceIcon>
          <AppWindow className="size-4" aria-hidden="true" />
        </ServiceIcon>
        <div className="min-w-0 flex-1">
          <div className="truncate font-medium">{app.name}</div>
          <div className="flex items-center gap-1.5 text-[12.5px] text-muted-foreground">
            <Source className="size-3.5 shrink-0" aria-hidden="true" />
            <span className="truncate font-mono text-[12px]">{source}</span>
          </div>
        </div>
        <span className="hidden max-w-56 truncate text-[13px] text-muted-foreground md:block">
          {app.domains[0] ?? ''}
          {app.domains.length > 1 ? ` +${app.domains.length - 1}` : ''}
        </span>
        <span className="hidden w-24 text-right text-[13px] text-muted-foreground lg:block">{timeAgo(app.updated_at)}</span>
        <Status tone={look.tone} className="w-28 justify-end">
          {look.label}
        </Status>
        <ChevronRight className="size-4 shrink-0 text-muted-foreground/60 transition-transform group-hover:translate-x-0.5" aria-hidden="true" />
      </Link>
    </li>
  )
}

function DbRow({ db }: { db: Db }) {
  const look = DATABASE_STATUS[db.status]
  const address = `${db.host}:${db.port}`
  return (
    <li className="flex items-center gap-3 px-4 py-3">
      <ServiceIcon>
        <EngineMark engine={db.engine} className="size-4" />
      </ServiceIcon>
      <div className="min-w-0 flex-1">
        <div className="truncate font-medium">{db.name}</div>
        <div className="flex items-center gap-1 text-[12.5px] text-muted-foreground">
          <span className="truncate font-mono text-[12px]">{address}</span>
          <CopyButton value={address} label="Copy address" />
        </div>
      </div>
      <span className="hidden text-[13px] text-muted-foreground md:block">
        {db.engine} {db.version}
      </span>
      <Status tone={look.tone} className="w-28 justify-end">
        {look.label}
      </Status>
      <Link
        to="/databases"
        className="text-[13px] text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
        aria-label={`Manage ${db.name}`}
      >
        Manage
      </Link>
    </li>
  )
}
