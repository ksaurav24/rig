import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Link, createFileRoute, useNavigate } from '@tanstack/react-router'
import { ArrowUpRight, HardDrive, KeyRound, Search } from 'lucide-react'
import { useDeferredValue, useMemo, useState, type FormEvent } from 'react'
import { toast } from 'sonner'
import { api, unwrap, type Schemas } from '@/api/client'
import { meQuery, projectsQuery } from '@/api/queries'
import { Page } from '@/components/app/page'
import { EmptyState } from '@/components/app/state'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Field, FieldDescription, FieldGroup, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Spinner } from '@/components/ui/spinner'
import { CATALOG, CATALOG_CATEGORIES, type CatalogCategory, type CatalogTemplate } from '@/lib/catalog'
import { hasRole } from '@/lib/nav'
import { cn } from '@/lib/utils'

export const Route = createFileRoute('/_app/catalog')({
  staticData: { crumb: 'Catalog' },
  // ?project=<id> arrives from a project's "Create service" menu and
  // preselects that project in the deploy form.
  validateSearch: (search: Record<string, unknown>): { project?: string } =>
    typeof search.project === 'string' ? { project: search.project } : {},
  loader: ({ context }) => {
    void context.queryClient.prefetchQuery(projectsQuery)
  },
  component: CatalogPage,
})

function CatalogPage() {
  const { data: me } = useQuery(meQuery)
  const canDeploy = me ? hasRole(me.role, 'developer') : false
  const [category, setCategory] = useState<CatalogCategory | 'all'>('all')
  const [query, setQuery] = useState('')
  const needle = useDeferredValue(query.trim().toLowerCase())
  const [picked, setPicked] = useState<CatalogTemplate | null>(null)

  const shown = useMemo(
    () =>
      CATALOG.filter(
        (t) =>
          (category === 'all' || t.category === category) &&
          (!needle || `${t.name} ${t.description} ${t.category}`.toLowerCase().includes(needle)),
      ),
    [category, needle],
  )

  return (
    <Page
      title="Catalog"
      description="Ready-made apps that run as a single container. Rig fills in the image, port, storage and secrets."
    >
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div className="-mx-4 flex gap-1.5 overflow-x-auto px-4 sm:mx-0 sm:flex-wrap sm:px-0" role="group" aria-label="Category">
          {(['all', ...CATALOG_CATEGORIES] as const).map((c) => (
            <Button
              key={c}
              size="sm"
              variant={category === c ? 'secondary' : 'ghost'}
              aria-pressed={category === c}
              className={cn('shrink-0', category === c && 'ring-1 ring-border')}
              onClick={() => setCategory(c)}
            >
              {c === 'all' ? 'All' : c}
            </Button>
          ))}
        </div>
        <div className="relative sm:w-64">
          <Search className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            type="search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search apps"
            aria-label="Search apps"
            className="pl-8"
          />
        </div>
      </div>

      {shown.length === 0 ? (
        <EmptyState icon={Search} title="No apps match">
          Try another word, or deploy any image from a project.
        </EmptyState>
      ) : (
        <ul className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {shown.map((t) => (
            <li key={t.id}>
              <TemplateCard template={t} canDeploy={canDeploy} onDeploy={() => setPicked(t)} />
            </li>
          ))}
        </ul>
      )}

      <DeployDialog template={picked} onClose={() => setPicked(null)} />
    </Page>
  )
}

function Monogram({ name }: { name: string }) {
  return (
    <span
      aria-hidden
      className="grid size-9 shrink-0 place-items-center rounded-md border bg-muted font-mono text-sm font-semibold text-foreground"
    >
      {name.slice(0, 2)}
    </span>
  )
}

function TemplateCard({
  template: t,
  canDeploy,
  onDeploy,
}: {
  template: CatalogTemplate
  canDeploy: boolean
  onDeploy: () => void
}) {
  const secrets = t.variables.filter((v) => v.generate).length
  return (
    <article className="flex h-full flex-col gap-3 rounded-lg border bg-card p-4 transition-colors hover:border-foreground/20">
      <header className="flex items-start gap-3">
        <Monogram name={t.name} />
        <div className="min-w-0 flex-1">
          <h2 className="truncate font-medium">{t.name}</h2>
          <p className="text-[0.8125rem] text-muted-foreground">{t.category}</p>
        </div>
        <a
          href={t.website}
          target="_blank"
          rel="noreferrer noopener"
          className="rounded-sm p-1 text-muted-foreground hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
          aria-label={`${t.name} website`}
        >
          <ArrowUpRight className="size-4" />
        </a>
      </header>
      <p className="flex-1 text-muted-foreground">{t.description}</p>
      <footer className="flex items-center justify-between gap-2">
        <div className="flex min-w-0 items-center gap-3 text-xs text-muted-foreground">
          <span className="truncate font-mono" title={t.image}>
            {t.image}
          </span>
          {t.volumes.length ? (
            <span className="flex shrink-0 items-center gap-1" title="Keeps data in a persistent volume">
              <HardDrive className="size-3.5" />
              <span className="sr-only">Persistent storage</span>
            </span>
          ) : null}
          {secrets ? (
            <span className="flex shrink-0 items-center gap-1" title="Rig generates its secrets">
              <KeyRound className="size-3.5" />
              <span className="sr-only">Generated secrets</span>
            </span>
          ) : null}
        </div>
        {canDeploy ? (
          <Button size="sm" variant="outline" onClick={onDeploy} aria-label={`Deploy ${t.name}`}>
            Deploy
          </Button>
        ) : null}
      </footer>
    </article>
  )
}

/** 32 URL-safe characters from the browser's CSPRNG. */
function generatePassword(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(24))
  return btoa(String.fromCharCode(...bytes))
    .replaceAll('+', '-')
    .replaceAll('/', '_')
}

function DeployDialog({ template, onClose }: { template: CatalogTemplate | null; onClose: () => void }) {
  return (
    <Dialog open={template !== null} onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-lg">
        {template ? <DeployForm key={template.id} template={template} onClose={onClose} /> : null}
      </DialogContent>
    </Dialog>
  )
}

function DeployForm({ template: t, onClose }: { template: CatalogTemplate; onClose: () => void }) {
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const { data: projects, isPending } = useQuery(projectsQuery)
  const { project: preset } = Route.useSearch()
  const [projectId, setProjectId] = useState<string | undefined>(
    () => preset,
  )
  const project =
    (projectId && projects?.some((p) => p.id === projectId) ? projectId : undefined) ?? projects?.[0]?.id
  const projectItems = (projects ?? []).map((p) => ({ value: p.id, label: p.name }))
  const asked = t.variables.filter((v) => v.required && v.value === undefined && !v.generate)

  const deploy = useMutation({
    mutationFn: async (body: Schemas['CreateAppRequest']) => {
      const app = await unwrap(api.POST('/api/v1/apps', { body }))
      const deployed = await unwrap(api.POST('/api/v1/apps/{id}/deploy', { params: { path: { id: app.id } } })).then(
        () => true,
        () => false,
      )
      return { app, deployed }
    },
    onSuccess: async ({ app, deployed }) => {
      onClose()
      if (deployed) toast.success(`${app.name} created. Deploying now`)
      else toast.warning(`${app.name} created, but the first deploy did not start. Use Redeploy to try again.`)
      void queryClient.invalidateQueries({ queryKey: ['apps'] })
      void queryClient.invalidateQueries({ queryKey: ['projects'] })
      await navigate({ to: deployed ? '/apps/$appId/deployments' : '/apps/$appId', params: { appId: app.id } })
    },
  })

  function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault()
    if (!project) return
    const form = new FormData(e.currentTarget)
    const env = t.variables.flatMap((v) => {
      const value = v.generate ? generatePassword() : (v.value ?? String(form.get(`var-${v.key}`) ?? '').trim())
      return value ? [{ key: v.key, value, is_secret: Boolean(v.secret || v.generate) }] : []
    })
    deploy.mutate({
      project_id: project,
      name: String(form.get('name') ?? '').trim(),
      source_kind: 'image',
      image: t.image,
      port: t.port,
      memory_mb: t.memoryMb,
      health_path: t.healthPath,
      volumes: [...t.volumes],
      env,
    })
  }

  if (!isPending && !projects?.length) {
    return (
      <>
        <DialogHeader>
          <DialogTitle>Deploy {t.name}</DialogTitle>
          <DialogDescription>Apps live in a project. Create one first, then come back here.</DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button nativeButton={false} render={<Link to="/projects" />} onClick={onClose}>
            Go to projects
          </Button>
        </DialogFooter>
      </>
    )
  }

  const generated = t.variables.filter((v) => v.generate)
  return (
    <form onSubmit={submit} className="contents">
      <DialogHeader>
        <DialogTitle>Deploy {t.name}</DialogTitle>
        <DialogDescription>
          Runs <span className="font-mono text-foreground">{t.image}</span> with {t.memoryMb} MB of memory.
        </DialogDescription>
      </DialogHeader>
      <FieldGroup>
        <div className="grid gap-4 sm:grid-cols-2">
          <Field>
            <FieldLabel htmlFor="cat-name">Name</FieldLabel>
            <Input id="cat-name" name="name" required maxLength={64} defaultValue={t.id} autoComplete="off" autoFocus />
          </Field>
          <Field>
            <FieldLabel htmlFor="cat-project">Project</FieldLabel>
            <Select items={projectItems} value={project} onValueChange={(v) => setProjectId(v as string)}>
              <SelectTrigger id="cat-project" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {projectItems.map((p) => (
                  <SelectItem key={p.value} value={p.value}>
                    {p.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </Field>
        </div>
        {asked.map((v) => (
          <Field key={v.key}>
            <FieldLabel htmlFor={`var-${v.key}`}>{v.key}</FieldLabel>
            <Input
              id={`var-${v.key}`}
              name={`var-${v.key}`}
              required
              autoComplete="off"
              type={v.secret ? 'password' : 'text'}
            />
            {v.description ? <FieldDescription>{v.description}</FieldDescription> : null}
          </Field>
        ))}
        <dl className="grid gap-2 rounded-md border bg-muted/40 p-3 text-[0.8125rem]">
          <div className="flex gap-2">
            <dt className="w-28 shrink-0 text-muted-foreground">Storage</dt>
            <dd className="min-w-0 font-mono text-xs leading-5 break-all">
              {t.volumes.length ? t.volumes.join(', ') : 'None (stateless)'}
            </dd>
          </div>
          {generated.length ? (
            <div className="flex gap-2">
              <dt className="w-28 shrink-0 text-muted-foreground">Generated</dt>
              <dd className="min-w-0">
                <span className="font-mono text-xs">{generated.map((v) => v.key).join(', ')}</span>
                <span className="block text-muted-foreground">Stored encrypted. Reveal them in the app's Environment tab.</span>
              </dd>
            </div>
          ) : null}
        </dl>
      </FieldGroup>
      <DialogFooter>
        <Button type="button" variant="ghost" onClick={onClose}>
          Cancel
        </Button>
        <Button type="submit" disabled={deploy.isPending || !project}>
          {deploy.isPending ? <Spinner /> : null}
          Deploy
        </Button>
      </DialogFooter>
    </form>
  )
}
