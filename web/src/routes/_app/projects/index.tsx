import { useQuery } from '@tanstack/react-query'
import { Link, createFileRoute } from '@tanstack/react-router'
import { Boxes, Plus } from 'lucide-react'
import { useMemo, useState } from 'react'
import type { Schemas } from '@/api/client'
import { appsQuery, databasesQuery, meQuery, projectsQuery } from '@/api/queries'
import { Page } from '@/components/app/page'
import { NewProjectDialog } from '@/components/app/project-dialogs'
import { EmptyState, ErrorState, type Tone } from '@/components/app/state'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { timeAgo } from '@/lib/format'
import { hasRole } from '@/lib/nav'
import { APP_STATUS, DATABASE_STATUS } from '@/lib/status'
import { cn } from '@/lib/utils'

type Project = Schemas['Project']

export const Route = createFileRoute('/_app/projects/')({
  staticData: { crumb: 'Projects' },
  loader: ({ context }) => {
    void context.queryClient.prefetchQuery(projectsQuery)
    void context.queryClient.prefetchQuery(appsQuery)
    void context.queryClient.prefetchQuery(databasesQuery)
  },
  component: ProjectsPage,
})

interface Summary {
  apps: number
  databases: number
  /** Worst tone among the project's services, for the corner dot. */
  tone: Tone | null
  issues: number
  lastChange: string | null
}

const TONE_RANK: Record<Tone, number> = { fail: 4, warn: 3, busy: 2, ok: 1, idle: 0 }

function ProjectsPage() {
  const projects = useQuery(projectsQuery)
  const apps = useQuery(appsQuery)
  const databases = useQuery(databasesQuery)
  const { data: me } = useQuery(meQuery)
  const canEdit = me ? hasRole(me.role, 'developer') : false
  const [creating, setCreating] = useState(false)

  const summaries = useMemo(() => {
    const map = new Map<string, Summary>()
    const get = (id: string) => {
      let s = map.get(id)
      if (!s) map.set(id, (s = { apps: 0, databases: 0, tone: null, issues: 0, lastChange: null }))
      return s
    }
    const note = (s: Summary, tone: Tone, at: string | null | undefined) => {
      if (s.tone === null || TONE_RANK[tone] > TONE_RANK[s.tone]) s.tone = tone
      if (tone === 'fail' || tone === 'warn') s.issues += 1
      if (at && (!s.lastChange || at > s.lastChange)) s.lastChange = at
    }
    for (const app of apps.data ?? []) {
      const s = get(app.project_id)
      s.apps += 1
      note(s, APP_STATUS[app.status].tone, app.updated_at)
    }
    for (const db of databases.data ?? []) {
      const s = get(db.project_id)
      s.databases += 1
      note(s, DATABASE_STATUS[db.status].tone, db.created_at)
    }
    return map
  }, [apps.data, databases.data])

  const newProject = canEdit ? (
    <Button onClick={() => setCreating(true)}>
      <Plus data-icon="inline-start" />
      New project
    </Button>
  ) : undefined

  const pending = projects.isPending
  const error = projects.error

  return (
    <Page
      title="Projects"
      description="Each project groups the apps and databases behind one product."
      actions={projects.data?.length ? newProject : undefined}
    >
      {pending ? (
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {Array.from({ length: 3 }, (_, i) => (
            <Skeleton key={i} className="h-[132px] rounded-lg" />
          ))}
        </div>
      ) : error ? (
        <ErrorState error={error} onRetry={() => void projects.refetch()} />
      ) : projects.data!.length === 0 ? (
        <EmptyState icon={Boxes} title="No projects yet" action={newProject}>
          {canEdit
            ? 'Start with a project, then add apps and databases to it. Services in one project share a private network.'
            : 'Nothing has been set up yet. Ask an admin or developer to create a project.'}
        </EmptyState>
      ) : (
        <ul className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {projects.data!.map((project) => (
            <li key={project.id}>
              <ProjectCard project={project} summary={summaries.get(project.id)} loading={apps.isPending || databases.isPending} />
            </li>
          ))}
        </ul>
      )}

      <NewProjectDialog open={creating} onOpenChange={setCreating} />
    </Page>
  )
}

const DOT: Record<Tone, string> = {
  ok: 'bg-ok',
  busy: 'bg-busy animate-pulse',
  warn: 'bg-warn',
  fail: 'bg-fail',
  idle: 'bg-muted-foreground/40',
}

function ProjectCard({ project, summary, loading }: { project: Project; summary?: Summary; loading: boolean }) {
  const apps = summary?.apps ?? project.app_count
  const dbs = summary?.databases ?? 0
  const empty = !loading && apps === 0 && dbs === 0
  const counts = [
    `${apps} app${apps === 1 ? '' : 's'}`,
    loading ? null : `${dbs} database${dbs === 1 ? '' : 's'}`,
  ].filter(Boolean)

  return (
    <Link
      to="/projects/$projectId"
      params={{ projectId: project.id }}
      className={cn(
        'flex h-full min-h-[132px] flex-col rounded-lg bg-card p-4 outline-none',
        'shadow-[0_0_0_1px_var(--border)] transition-[box-shadow,background-color] duration-150',
        'hover:shadow-[0_0_0_1px_var(--ring)] focus-visible:ring-1 focus-visible:ring-[color:var(--focus-ring)]',
      )}
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="truncate font-medium">{project.name}</div>
          <div className="mt-0.5 line-clamp-2 text-[13px] leading-snug text-muted-foreground">
            {project.description || <span className="font-mono text-[12px]">{project.slug}</span>}
          </div>
        </div>
        {summary?.tone ? (
          <span
            className={cn('mt-1.5 size-2 shrink-0 rounded-full', DOT[summary.tone])}
            aria-label={summary.issues ? `${summary.issues} need attention` : 'All healthy'}
          />
        ) : null}
      </div>
      <div className="mt-auto flex items-center justify-between gap-3 pt-5 text-[12.5px] text-muted-foreground">
        {empty ? (
          <span>Empty. Add the first service</span>
        ) : (
          <span className="tabular-nums">
            {counts.join(' · ')}
            {summary?.issues ? <span className="text-fail"> · {summary.issues} need attention</span> : null}
          </span>
        )}
        <span className="shrink-0">{timeAgo(summary?.lastChange ?? project.created_at)}</span>
      </div>
    </Link>
  )
}
