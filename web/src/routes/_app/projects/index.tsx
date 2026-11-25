import { useQuery } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { Boxes, Pin, PinOff, Plus } from 'lucide-react'
import { useMemo, useState } from 'react'
import type { Schemas } from '@/api/client'
import { appsQuery, databasesQuery, meQuery, projectsQuery } from '@/api/queries'
import { Page } from '@rig/ui/app/page'
import { NewProjectDialog } from '@/components/app/project-dialogs'
import { EmptyState, ErrorState, Status, type Tone } from '@/components/app/state'
import { Card, CardGroup } from '@rig/ui/card'
import { Tooltip } from '@rig/ui/tooltip'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { timeAgo } from '@/lib/format'
import { hasRole } from '@/lib/nav'
import { usePreferences } from '@/lib/preferences'
import { APP_STATUS, DATABASE_STATUS } from '@/lib/status'
import { cn } from '@rig/ui/lib/utils'

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
  /** Worst tone among the project's services. */
  tone: Tone | null
  issues: number
  lastChange: string | null
}

const TONE_RANK: Record<Tone, number> = { fail: 4, warn: 3, busy: 2, ok: 1, idle: 0 }

function normalizedSlug(name: string) {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
}

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
    <Page title="Projects" actions={projects.data?.length ? newProject : undefined}>
      {pending ? (
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {Array.from({ length: 3 }, (_, i) => (
            <Skeleton key={i} className="h-[118px] rounded-xl" />
          ))}
        </div>
      ) : error ? (
        <ErrorState error={error} onRetry={() => void projects.refetch()} />
      ) : projects.data!.length === 0 ? (
        <EmptyState icon={Boxes} title="No projects yet" action={newProject}>
          {canEdit
            ? 'Create a project to group the apps and databases of a service on a shared private network.'
            : 'Ask an admin or developer to create a project.'}
        </EmptyState>
      ) : (
        <CardGroup
          columns={3}
          border="outlined"
          separated
          className="grid-cols-1 sm:grid-cols-2 lg:grid-cols-3"
        >
          {projects.data!.map((project, index) => (
            <ProjectCard
              key={project.id}
              index={index}
              project={project}
              summary={summaries.get(project.id)}
              loading={apps.isPending || databases.isPending}
            />
          ))}
        </CardGroup>
      )}

      <NewProjectDialog open={creating} onOpenChange={setCreating} />
    </Page>
  )
}

function statusLabel(summary: Summary | undefined, empty: boolean): { tone: Tone; label: string } | null {
  if (empty) return { tone: 'idle', label: 'Empty' }
  if (!summary?.tone) return null
  if (summary.issues > 0) return { tone: summary.tone, label: `${summary.issues} issue${summary.issues === 1 ? '' : 's'}` }
  if (summary.tone === 'busy') return { tone: 'busy', label: 'Deploying' }
  if (summary.tone === 'ok') return { tone: 'ok', label: 'Healthy' }
  return { tone: 'idle', label: 'Idle' }
}

function ProjectCard({
  index,
  project,
  summary,
  loading,
}: {
  index: number
  project: Project
  summary?: Summary
  loading: boolean
}) {
  const { isProjectPinned, togglePinnedProject } = usePreferences()
  const pinned = isProjectPinned(project.id)
  const apps = summary?.apps ?? project.app_count
  const dbs = summary?.databases ?? 0
  const empty = !loading && apps === 0 && dbs === 0
  const counts = [
    `${apps} app${apps === 1 ? '' : 's'}`,
    loading ? null : `${dbs} database${dbs === 1 ? '' : 's'}`,
  ].filter(Boolean)
  const showSlug = !project.description && project.slug !== normalizedSlug(project.name)
  const state = statusLabel(summary, empty)

  return (
    <Card
      index={index}
      href={`/projects/${project.id}`}
      label={project.name}
      className="group/project flex h-full min-h-[116px] flex-col justify-between gap-4 bg-card p-4"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="truncate text-[14.5px] font-medium text-foreground">{project.name}</div>
          {project.description ? (
            <div className="mt-1 line-clamp-2 text-[12.5px] leading-snug text-muted-foreground">
              {project.description}
            </div>
          ) : showSlug ? (
            <div className="mt-0.5 truncate font-mono text-[12px] text-muted-foreground">{project.slug}</div>
          ) : null}
        </div>
        <div className="flex shrink-0 items-center gap-1.5">
          {state ? (
            <Status tone={state.tone} variant="ghost" className="shrink-0 text-[12px] text-muted-foreground">
              {state.label}
            </Status>
          ) : null}
          <Tooltip content={pinned ? 'Unpin from sidebar' : 'Pin to sidebar'}>
            <button
              type="button"
              aria-label={pinned ? `Unpin ${project.name}` : `Pin ${project.name}`}
              aria-pressed={pinned}
              onClick={(e) => {
                e.preventDefault()
                e.stopPropagation()
                togglePinnedProject(project.id)
              }}
              className={cn(
                'relative z-10 grid size-6 place-items-center rounded-md text-muted-foreground transition-opacity hover:bg-hover hover:text-foreground',
                pinned ? 'text-primary opacity-100' : 'opacity-0 group-hover/project:opacity-100 focus-visible:opacity-100',
              )}
            >
              {pinned ? <PinOff className="size-3.5" /> : <Pin className="size-3.5" />}
            </button>
          </Tooltip>
        </div>
      </div>

      <div className="flex items-center justify-between gap-3 text-[12.5px] text-muted-foreground">
        {empty ? (
          <span>Add your first service</span>
        ) : (
          <span className="tabular-nums">{counts.join(' · ')}</span>
        )}
        <span className="shrink-0 text-[12px]">{timeAgo(summary?.lastChange ?? project.created_at)}</span>
      </div>
    </Card>
  )
}
