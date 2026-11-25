import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Link, createFileRoute, useNavigate } from '@tanstack/react-router'
import { ArrowUpRight, HardDrive, KeyRound, Search } from 'lucide-react'
import { useDeferredValue, useMemo, useState, type FormEvent } from 'react'
import { toast } from 'sonner'
import { api, unwrap, type Schemas } from '@/api/client'
import { meQuery, projectsQuery } from '@/api/queries'
import { Page } from '@rig/ui/app/page'
import { EmptyState } from '@/components/app/state'
import { Badge } from '@rig/ui/badge'
import { Card, CardGroup } from '@rig/ui/card'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@rig/ui/dialog'
import { Field, FieldDescription, FieldGroup, FieldLabel } from '@rig/ui/field'
import { Input } from '@rig/ui/input'
import { Select, SelectContent, SelectItem, SelectTrigger } from '@rig/ui/select'
import { Tabs, TabsItem } from '@rig/ui/tabs'
import { Tooltip } from '@rig/ui/tooltip'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import { CATALOG, CATALOG_CATEGORIES, type CatalogCategory, type CatalogTemplate } from '@/lib/catalog'
import { hasRole } from '@/lib/nav'

const CATEGORIES = ['all', ...CATALOG_CATEGORIES] as const

export const Route = createFileRoute('/_app/catalog')({
  staticData: { crumb: 'Catalog' },
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

  const catIndex = CATEGORIES.indexOf(category)

  return (
    <Page title="Catalog">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div className="-mx-4 overflow-x-auto px-4 sm:mx-0 sm:px-0">
          <Tabs
            size="compact"
            selectedIndex={catIndex}
            onSelect={(i) => setCategory(CATEGORIES[i])}
            aria-label="Category"
            className="w-fit border border-border/80 bg-card/70"
          >
            {CATEGORIES.map((c, i) => (
              <TabsItem key={c} index={i} label={c === 'all' ? 'All' : c} />
            ))}
          </Tabs>
        </div>
        <div className="relative sm:w-64">
          <Search className="pointer-events-none absolute top-1/2 left-2.5 z-10 size-3.5 -translate-y-1/2 text-muted-foreground" />
          <Input
            type="search"
            size="compact"
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
          Try another search term, or deploy any image directly from a project.
        </EmptyState>
      ) : (
        <CardGroup
          columns={3}
          border="outlined"
          separated
          className="grid-cols-1 sm:grid-cols-2 lg:grid-cols-3"
        >
          {shown.map((t, index) => (
            <TemplateCard key={t.id} index={index} template={t} canDeploy={canDeploy} onDeploy={() => setPicked(t)} />
          ))}
        </CardGroup>
      )}

      <DeployDialog template={picked} onClose={() => setPicked(null)} />
    </Page>
  )
}

function Monogram({ name }: { name: string }) {
  return (
    <span
      aria-hidden
      className="grid size-9 shrink-0 place-items-center rounded-lg border border-border/80 bg-surface font-mono text-xs font-semibold tracking-wider text-foreground uppercase"
    >
      {name.slice(0, 2)}
    </span>
  )
}

function TemplateCard({
  index,
  template: t,
  canDeploy,
  onDeploy,
}: {
  index: number
  template: CatalogTemplate
  canDeploy: boolean
  onDeploy: () => void
}) {
  const secrets = t.variables.filter((v) => v.generate).length
  return (
    <Card index={index} className="flex h-full flex-col justify-between gap-3 bg-card p-4">
      <div className="flex flex-col gap-3">
        <header className="flex items-start gap-3">
          <Monogram name={t.name} />
          <div className="min-w-0 flex-1">
            <h2 className="truncate text-[14px] font-medium text-foreground">{t.name}</h2>
            <p className="text-[12px] text-muted-foreground">{t.category}</p>
          </div>
          <Tooltip content={`Open ${t.name} website`}>
            <a
              href={t.website}
              target="_blank"
              rel="noreferrer noopener"
              className="relative z-10 rounded-md p-1 text-muted-foreground transition-colors hover:bg-hover hover:text-foreground focus-visible:ring-1 focus-visible:ring-[color:var(--focus-ring)] focus-visible:outline-none"
              aria-label={`${t.name} website`}
            >
              <ArrowUpRight className="size-4" />
            </a>
          </Tooltip>
        </header>
        <p className="text-[13px] leading-relaxed text-muted-foreground">{t.description}</p>
      </div>

      <footer className="flex flex-col gap-2.5 pt-2 border-t border-border/60">
        <div className="flex items-center justify-between gap-2">
          <span className="min-w-0 truncate font-mono text-[11.5px] text-muted-foreground" title={t.image}>
            {t.image}
          </span>
          {canDeploy ? (
            <Button
              size="sm"
              variant="outline"
              onClick={onDeploy}
              aria-label={`Deploy ${t.name}`}
              className="relative z-10 shrink-0"
            >
              Deploy
            </Button>
          ) : null}
        </div>
        {(t.volumes.length > 0 || secrets > 0) ? (
          <div className="flex flex-wrap items-center gap-1.5">
            {t.volumes.length > 0 ? (
              <Tooltip content="Persistent storage volume configured automatically">
                <Badge
                  variant="solid"
                  color="gray"
                  size="compact"
                  icon={HardDrive}
                  className="relative z-10 font-normal text-muted-foreground"
                >
                  Storage
                </Badge>
              </Tooltip>
            ) : null}
            {secrets > 0 ? (
              <Tooltip content="Secrets generated automatically on deploy">
                <Badge
                  variant="solid"
                  color="gray"
                  size="compact"
                  icon={KeyRound}
                  className="relative z-10 font-normal text-muted-foreground"
                >
                  Auto-secrets
                </Badge>
              </Tooltip>
            ) : null}
          </div>
        ) : null}
      </footer>
    </Card>
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
      <DialogContent size="lg">
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
  const [projectId, setProjectId] = useState<string | undefined>(() => preset)
  const project =
    (projectId && projects?.some((p) => p.id === projectId) ? projectId : undefined) ?? projects?.[0]?.id
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
          Runs <span className="font-mono text-foreground">{t.image}</span> · {t.memoryMb} MB RAM.
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
            <Select value={project ?? ''} onValueChange={(v) => setProjectId(v)}>
              <SelectTrigger id="cat-project" className="w-full" />
              <SelectContent>
                {(projects ?? []).map((p, i) => (
                  <SelectItem key={p.id} index={i} value={p.id}>
                    {p.name}
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
        <dl className="grid gap-2 rounded-lg border border-border/80 bg-surface/60 p-3 text-[12.5px]">
          <div className="flex gap-2">
            <dt className="w-24 shrink-0 text-muted-foreground">Storage</dt>
            <dd className="min-w-0 font-mono text-xs leading-5 break-all">
              {t.volumes.length ? t.volumes.join(', ') : 'Stateless'}
            </dd>
          </div>
          {generated.length ? (
            <div className="flex gap-2">
              <dt className="w-24 shrink-0 text-muted-foreground">Secrets</dt>
              <dd className="min-w-0">
                <span className="font-mono text-xs">{generated.map((v) => v.key).join(', ')}</span>
                <span className="block text-muted-foreground">Generated &amp; encrypted; reveal under Environment.</span>
              </dd>
            </div>
          ) : null}
        </dl>
      </FieldGroup>
      <DialogFooter>
        <Button type="button" variant="outline" onClick={onClose}>
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
