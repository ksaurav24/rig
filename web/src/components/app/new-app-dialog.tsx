import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Link, useNavigate } from '@tanstack/react-router'
import { ArrowLeft, ExternalLink, GitBranch, Globe, Lock, RefreshCw, ShieldCheck } from 'lucide-react'
import { useMemo, useState, type FormEvent, type ReactNode } from 'react'
import { toast } from 'sonner'
import { api, ApiError, unwrap, type Schemas } from '@/api/client'
import { githubBranchesQuery, githubQuery, githubReposQuery, meQuery } from '@/api/queries'
import { BrandMark } from '@/components/app/service-icons'
import { Accordion, AccordionContent, AccordionItem, AccordionTrigger } from '@rig/ui/accordion'
import { Banner, BannerDescription, BannerTitle } from '@rig/ui/banner'
import { Checkbox } from '@rig/ui/checkbox'
import { Combobox, ComboboxContent, ComboboxEmpty, ComboboxInput, ComboboxItem, ComboboxList } from '@rig/ui/combobox'
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
import { Switch } from '@rig/ui/switch'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { keepComboboxEnter } from '@/lib/forms'
import { hasRole } from '@/lib/nav'
import { cn } from '@rig/ui/lib/utils'

type Provider = 'github' | 'git' | 'image'
type Build = 'auto' | 'dockerfile'
type Repo = Schemas['GithubRepo']

const DEFAULT_DOCKERFILE = 'Dockerfile'

const PROVIDERS: { id: Provider; mark: 'github' | 'git' | 'docker'; title: string; hint: string }[] = [
  { id: 'github', mark: 'github', title: 'GitHub', hint: 'Pick a repo; deploy on push' },
  { id: 'git', mark: 'git', title: 'Git URL', hint: 'Any public https:// or ssh:// repo' },
  { id: 'image', mark: 'docker', title: 'Docker image', hint: 'Pull a prebuilt image' },
]

/** Lowercase, dash-separated, as the API accepts for app names. */
function suggestName(source: string) {
  const base = source.split('/').pop()?.replace(/\.git$/, '').split(':')[0] ?? ''
  return base
    .toLowerCase()
    .replace(/[^a-z0-9-]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 64)
}

export function NewAppDialog({ project, onClose }: { project: Schemas['Project'] | null; onClose: () => void }) {
  return (
    <Dialog open={project !== null} onOpenChange={(open) => (open ? undefined : onClose())}>
      <DialogContent size="lg" className="gap-5">
        {project ? <NewAppForm key={project.id} project={project} onClose={onClose} /> : null}
      </DialogContent>
    </Dialog>
  )
}

function NewAppForm({ project, onClose }: { project: Schemas['Project']; onClose: () => void }) {
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const [provider, setProvider] = useState<Provider | null>(null)
  const [repo, setRepo] = useState<Repo | null>(null)
  const [branch, setBranch] = useState('')
  const [build, setBuild] = useState<Build>('auto')
  const [name, setName] = useState('')
  const [nameTouched, setNameTouched] = useState(false)
  const [deployNow, setDeployNow] = useState(true)
  const [autoDeploy, setAutoDeploy] = useState(true)
  const github = useQuery({ ...githubQuery, enabled: provider === 'github' })
  const githubMissing = provider === 'github' && github.isSuccess && !github.data.app

  const create = useMutation({
    mutationFn: async (body: Schemas['CreateAppRequest']) => {
      let app = await unwrap(api.POST('/api/v1/apps', { body }))
      if (provider === 'github' && autoDeploy) {
        app = await unwrap(
          api.PATCH('/api/v1/apps/{id}', { params: { path: { id: app.id } }, body: { auto_deploy: true } }),
        ).catch(() => app)
      }
      if (!deployNow) return { app, deployed: null }
      const deployed = await unwrap(api.POST('/api/v1/apps/{id}/deploy', { params: { path: { id: app.id } } })).then(
        () => true,
        () => false,
      )
      return { app, deployed }
    },
    onSuccess: async ({ app, deployed }) => {
      onClose()
      if (deployed === null) toast.success(`${app.name} created. Deploy it when you are ready`)
      else if (deployed) toast.success(`${app.name} created. Deploying now`)
      else toast.warning(`${app.name} created, but the first deploy did not start. Use Redeploy to try again.`)
      void queryClient.invalidateQueries({ queryKey: ['apps'] })
      void queryClient.invalidateQueries({ queryKey: ['projects'] })
      await navigate({ to: deployed ? '/apps/$appId/deployments' : '/apps/$appId', params: { appId: app.id } })
    },
  })

  function autoName(source: string) {
    if (!nameTouched) setName(suggestName(source))
  }

  function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault()
    if (!provider) return
    const form = new FormData(e.currentTarget)
    const text = (key: string) => String(form.get(key) ?? '').trim()
    const dockerfile = build === 'dockerfile' ? text('dockerfile_path') || DEFAULT_DOCKERFILE : DEFAULT_DOCKERFILE
    const common = {
      project_id: project.id,
      name: name.trim(),
      port: Number(text('port')),
      volumes: text('volumes').split(/[\s,]+/).filter(Boolean),
      health_path: text('health_path') || null,
    }
    if (provider === 'image') {
      create.mutate({ ...common, source_kind: 'image', image: text('image') })
    } else {
      const url = provider === 'github' ? repo?.clone_url : text('git_url')
      if (!url) return
      create.mutate({
        ...common,
        source_kind: 'git',
        git_url: url,
        git_branch: branch.trim() || null,
        dockerfile_path: dockerfile,
      })
    }
  }

  if (!provider) {
    return (
      <>
        <DialogHeader>
          <DialogTitle>New app</DialogTitle>
          <DialogDescription>In {project.name}. Choose a source.</DialogDescription>
        </DialogHeader>
        <div className="grid gap-2.5 sm:grid-cols-3">
          {PROVIDERS.map((p) => (
            <button
              key={p.id}
              type="button"
              onClick={() => setProvider(p.id)}
              className={cn(
                'group flex flex-col items-start gap-3 rounded-lg bg-card p-4 text-left outline-none',
                'shadow-[0_0_0_1px_var(--border)] transition-[box-shadow,background-color] duration-150',
                'hover:bg-hover hover:shadow-[0_0_0_1px_var(--ring)] focus-visible:ring-1 focus-visible:ring-[color:var(--focus-ring)]',
              )}
            >
              <BrandMark name={p.mark} className="size-5" />
              <span>
                <span className="block font-medium text-foreground">{p.title}</span>
                <span className="mt-0.5 block text-[12.5px] leading-snug text-muted-foreground">{p.hint}</span>
              </span>
            </button>
          ))}
        </div>
        <DialogFooter>
          <Button type="button" variant="outline" onClick={onClose}>
            Cancel
          </Button>
        </DialogFooter>
      </>
    )
  }

  const current = PROVIDERS.find((p) => p.id === provider)!
  const isGit = provider !== 'image'
  const sourceReady = provider !== 'github' || repo !== null

  return (
    <form onSubmit={submit} onKeyDown={keepComboboxEnter} className="contents">
      <DialogHeader>
        <div className="flex items-center gap-2">
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            aria-label="Choose another source"
            onClick={() => {
              setProvider(null)
              setRepo(null)
            }}
            className="-ml-1.5"
          >
            <ArrowLeft />
          </Button>
          <DialogTitle className="flex items-center gap-2">
            <BrandMark name={current.mark} className="size-4" />
            New app from {current.title}
          </DialogTitle>
        </div>
        <DialogDescription>In {project.name}.</DialogDescription>
      </DialogHeader>

      <FieldGroup className="max-h-[min(62vh,560px)] overflow-y-auto px-px pb-px">
        <Section title="Source">
          {provider === 'github' ? (
            <GithubSource
              onUseGitUrl={() => setProvider('git')}
              repo={repo}
              onRepo={(r) => {
                setRepo(r)
                setBranch(r?.default_branch ?? '')
                if (r) autoName(r.full_name)
              }}
            />
          ) : provider === 'git' ? (
            <Field>
              <FieldLabel htmlFor="app-git-url">Repository URL</FieldLabel>
              <Input
                id="app-git-url"
                name="git_url"
                required
                autoFocus
                placeholder="https://github.com/acme/api.git"
                autoComplete="off"
                spellCheck={false}
                className="font-mono text-[13px]"
                onChange={(e) => autoName(e.target.value)}
              />
              <FieldDescription>Public repos only. Use GitHub for private repositories.</FieldDescription>
            </Field>
          ) : (
            <Field>
              <FieldLabel htmlFor="app-image">Image</FieldLabel>
              <Input
                id="app-image"
                name="image"
                required
                autoFocus
                maxLength={255}
                placeholder="ghcr.io/acme/api:latest"
                autoComplete="off"
                spellCheck={false}
                className="font-mono text-[13px]"
                onChange={(e) => autoName(e.target.value)}
              />
            </Field>
          )}
          {isGit && !githubMissing ? (
            <BranchField id="app-git-branch" repo={provider === 'github' ? repo : null} github={provider === 'github'} value={branch} onChange={setBranch} />
          ) : null}
          {provider === 'github' && !githubMissing ? (
            <div className="flex items-center justify-between gap-4 rounded-lg border border-border/80 bg-surface/40 px-3 py-2.5 text-[13px]">
              <div className="min-w-0">
                <span className="block font-medium">Deploy on push</span>
                <span className="block text-[12px] text-muted-foreground">
                  Redeploy automatically when {branch.trim() ? <code className="font-mono">{branch.trim()}</code> : 'the branch'} updates.
                </span>
              </div>
              <Switch
                checked={autoDeploy}
                onToggle={() => setAutoDeploy((v) => !v)}
                aria-label="Deploy on push"
              />
            </div>
          ) : null}
        </Section>

        {githubMissing ? null : (
          <>
            {isGit ? (
              <Section title="Build">
                <div role="radiogroup" aria-label="Build type" className="grid gap-2 sm:grid-cols-2">
                  <Choice selected={build === 'auto'} onSelect={() => setBuild('auto')} title="Auto-detect">
                    Root Dockerfile if present, otherwise Railpack.
                  </Choice>
                  <Choice selected={build === 'dockerfile'} onSelect={() => setBuild('dockerfile')} title="Dockerfile">
                    Custom Dockerfile path inside the repo.
                  </Choice>
                </div>
                {build === 'dockerfile' ? (
                  <Field>
                    <FieldLabel htmlFor="app-dockerfile">Dockerfile path</FieldLabel>
                    <Input
                      id="app-dockerfile"
                      name="dockerfile_path"
                      required
                      placeholder="docker/api.Dockerfile"
                      autoComplete="off"
                      spellCheck={false}
                      className="font-mono text-[13px]"
                    />
                  </Field>
                ) : null}
              </Section>
            ) : null}

            <Section title="Runtime">
              <div className="grid gap-4 sm:grid-cols-[1fr_9rem]">
                <Field>
                  <FieldLabel htmlFor="app-name">Name</FieldLabel>
                  <Input
                    id="app-name"
                    required
                    maxLength={64}
                    value={name}
                    onChange={(e) => {
                      setName(e.target.value)
                      setNameTouched(true)
                    }}
                    placeholder="api"
                    autoComplete="off"
                    spellCheck={false}
                  />
                </Field>
                <Field>
                  <FieldLabel htmlFor="app-port">Port</FieldLabel>
                  <Input
                    id="app-port"
                    name="port"
                    type="number"
                    inputMode="numeric"
                    required
                    min={1}
                    max={65535}
                    defaultValue={provider === 'image' ? 80 : 3000}
                  />
                </Field>
              </div>
              <FieldDescription className="-mt-1.5">
                Listening port inside the container (also injected as <code className="font-mono">PORT</code>).
              </FieldDescription>

              <Accordion type="single" collapsible size="compact" className="rounded-lg border border-border/80">
                <AccordionItem value="advanced">
                  <AccordionTrigger className="px-3 text-[13px] font-medium">
                    Advanced — health check &amp; persistent paths
                  </AccordionTrigger>
                  <AccordionContent className="px-3 pb-3">
                    <div className="grid gap-4 pt-1">
                      <Field>
                        <FieldLabel htmlFor="app-health">Health check path</FieldLabel>
                        <Input id="app-health" name="health_path" placeholder="/healthz" autoComplete="off" spellCheck={false} className="font-mono text-[13px]" />
                        <FieldDescription>Optional. Waits for 2xx before routing traffic.</FieldDescription>
                      </Field>
                      <Field>
                        <FieldLabel htmlFor="app-volumes">Persistent paths</FieldLabel>
                        <Input id="app-volumes" name="volumes" placeholder="/data" autoComplete="off" spellCheck={false} className="font-mono text-[13px]" />
                        <FieldDescription>Comma-separated mount paths kept across deploys.</FieldDescription>
                      </Field>
                    </div>
                  </AccordionContent>
                </AccordionItem>
              </Accordion>
            </Section>
          </>
        )}
      </FieldGroup>

      <DialogFooter className="items-center sm:justify-between">
        {githubMissing ? <span /> : (
          <label className="flex cursor-pointer items-center gap-2 text-[13px] text-muted-foreground select-none">
            <Checkbox checked={deployNow} onCheckedChange={(v) => setDeployNow(v === true)} />
            Deploy right after creating
          </label>
        )}
        <div className="flex gap-2">
          <Button type="button" variant="outline" onClick={onClose}>
            Cancel
          </Button>
          {githubMissing ? null : (
            <Button type="submit" loading={create.isPending} disabled={create.isPending || !sourceReady || githubMissing}>
              {deployNow ? 'Create and deploy' : 'Create app'}
            </Button>
          )}
        </div>
      </DialogFooter>
    </form>
  )
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <fieldset className="grid gap-3.5">
      <legend className="mb-2 text-[12px] font-medium text-muted-foreground">{title}</legend>
      {children}
    </fieldset>
  )
}

function Choice({
  selected,
  onSelect,
  title,
  children,
}: {
  selected: boolean
  onSelect: () => void
  title: string
  children: ReactNode
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      onClick={onSelect}
      className={cn(
        'flex items-start gap-2.5 rounded-lg p-3 text-left outline-none transition-[box-shadow,background-color] duration-150',
        'shadow-[0_0_0_1px_var(--border)] hover:bg-hover focus-visible:ring-1 focus-visible:ring-[color:var(--focus-ring)]',
        selected && 'bg-tint shadow-[0_0_0_1.5px_var(--foreground)]',
      )}
    >
      <span
        aria-hidden="true"
        className={cn(
          'mt-0.5 grid size-3.5 shrink-0 place-items-center rounded-full shadow-[0_0_0_1.5px_var(--border)]',
          selected && 'shadow-[0_0_0_1.5px_var(--foreground)]',
        )}
      >
        {selected ? <span className="size-1.5 rounded-full bg-foreground" /> : null}
      </span>
      <span>
        <span className="block text-[13px] font-medium">{title}</span>
        <span className="mt-0.5 block text-[12px] leading-snug text-muted-foreground">{children}</span>
      </span>
    </button>
  )
}

export function GithubSource({
  repo,
  onRepo,
  onUseGitUrl,
}: {
  repo: Repo | null
  onRepo: (repo: Repo | null) => void
  onUseGitUrl: () => void
}) {
  const { data: me } = useQuery(meQuery)
  const admin = me ? hasRole(me.role, 'admin') : false
  const status = useQuery(githubQuery)
  const app = status.data?.app ?? null
  const repos = useQuery({ ...githubReposQuery, enabled: app !== null, refetchOnWindowFocus: true })

  const items = useMemo(
    () => (repos.data ?? []).map((r) => ({ value: r.full_name, label: r.full_name, repo: r })),
    [repos.data],
  )

  if (status.isPending) return <Skeleton className="h-[68px]" />
  if (status.isError) return <Notice tone="fail" title="Could not reach GitHub settings" action={<RetryButton onClick={() => status.refetch()} />} />

  if (!app) {
    return (
      <Notice
        title="No GitHub App connected"
        action={
          <div className="flex flex-wrap gap-2">
            {admin ? (
              <Button nativeButton={false} size="sm" render={<Link to="/settings/github" />}>
                Connect GitHub
              </Button>
            ) : null}
            <Button type="button" variant="outline" size="sm" onClick={onUseGitUrl}>
              Use a Git URL instead
            </Button>
          </div>
        }
      >
        {admin
          ? 'Connect a GitHub App once for the organization to pick private repositories and deploy on push.'
          : 'An admin needs to connect GitHub first. Public repositories still work via Git URL.'}
      </Notice>
    )
  }

  const account = (
    <div className="flex items-center gap-2.5 rounded-lg border border-border/80 bg-surface/40 px-3 py-2">
      <BrandMark name="github" className="size-4 shrink-0" />
      <div className="min-w-0 flex-1 leading-tight">
        <div className="truncate text-[13px] font-medium">{app.owner}</div>
        <div className="truncate text-[12px] text-muted-foreground">via {app.name}</div>
      </div>
      <a
        href={app.install_url}
        target="_blank"
        rel="noreferrer"
        className="inline-flex shrink-0 items-center gap-1 text-[12.5px] text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
      >
        Configure access
        <ExternalLink className="size-3" aria-hidden="true" />
      </a>
    </div>
  )

  let picker: ReactNode
  if (repos.isPending) {
    picker = <Skeleton className="h-9" />
  } else if (repos.isError) {
    const reason = repos.error instanceof ApiError ? repos.error.message : 'GitHub did not answer.'
    picker = <Notice tone="fail" title="Could not list repositories" action={<RetryButton onClick={() => repos.refetch()} />}>{reason}</Notice>
  } else if (items.length === 0) {
    picker = (
      <Notice
        title="No repositories granted yet"
        action={
          <a href={app.install_url} target="_blank" rel="noreferrer" className="font-medium text-foreground underline-offset-4 hover:underline">
            Choose repositories on GitHub
          </a>
        }
      >
        Grant repository access on GitHub, then return here. The list refreshes automatically.
      </Notice>
    )
  } else {
    picker = (
      <Combobox
        items={items}
        value={repo?.full_name ?? ''}
        onValueChange={(v) => onRepo(repos.data?.find((r) => r.full_name === v) ?? null)}
      >
        <ComboboxInput id="app-repo" placeholder={`Search ${items.length} repositories`} autoFocus />
        <ComboboxContent>
          <ComboboxEmpty>No repository matches. Missing one? Use “Configure access”.</ComboboxEmpty>
          <ComboboxList>
            {(item) => {
              const r = (item as (typeof items)[number]).repo
              const [owner, repoName] = r.full_name.split('/')
              return (
                <ComboboxItem key={r.full_name} value={r.full_name}>
                  <span className="flex w-full min-w-0 items-center gap-2">
                    <span className="min-w-0 flex-1 truncate">
                      <span className="text-muted-foreground">{owner}/</span>
                      <span className="font-medium">{repoName}</span>
                    </span>
                    {r.private ? (
                      <Lock className="size-3.5 shrink-0 text-muted-foreground" aria-label="Private" />
                    ) : (
                      <Globe className="size-3.5 shrink-0 text-muted-foreground/60" aria-label="Public" />
                    )}
                  </span>
                </ComboboxItem>
              )
            }}
          </ComboboxList>
        </ComboboxContent>
      </Combobox>
    )
  }

  return (
    <>
      <Field>
        <FieldLabel>Account</FieldLabel>
        {account}
      </Field>
      <Field>
        <div className="flex items-center justify-between">
          <FieldLabel htmlFor="app-repo">Repository</FieldLabel>
          {repos.isSuccess ? (
            <button
              type="button"
              onClick={() => repos.refetch()}
              className="inline-flex items-center gap-1 text-[12.5px] text-muted-foreground hover:text-foreground"
            >
              <RefreshCw className={cn('size-3', repos.isFetching && 'animate-spin')} aria-hidden="true" />
              Refresh
            </button>
          ) : null}
        </div>
        {picker}
      </Field>
    </>
  )
}

export function BranchField({
  id,
  name,
  repo,
  github,
  value,
  onChange,
}: {
  id: string
  name?: string
  repo: Repo | null
  github: boolean
  value: string
  onChange: (branch: string) => void
}) {
  const branches = useQuery({ ...githubBranchesQuery(repo?.full_name ?? ''), enabled: repo !== null })
  const items = useMemo(
    () => (branches.data ?? []).map((b) => ({ value: b.name, label: b.name, branch: b })),
    [branches.data],
  )
  const hidden = name ? <input type="hidden" name={name} value={value} /> : null

  if (github && !repo) {
    return (
      <Field>
        <FieldLabel htmlFor={id}>Branch</FieldLabel>
        <Input id={id} disabled placeholder="Pick a repository first" className="font-mono text-[13px]" />
      </Field>
    )
  }

  if (repo && branches.isPending) {
    return (
      <Field>
        <FieldLabel htmlFor={id}>Branch</FieldLabel>
        <Skeleton className="h-9" />
        {hidden}
      </Field>
    )
  }

  if (repo && branches.isSuccess && items.length > 0) {
    const known = items.some((i) => i.value === value)
    return (
      <Field>
        <FieldLabel htmlFor={id}>Branch</FieldLabel>
        <Combobox items={items} value={known ? value : ''} onValueChange={(v) => v && onChange(String(v))}>
          <ComboboxInput id={id} placeholder={`Search ${items.length} branch${items.length === 1 ? '' : 'es'}`} />
          <ComboboxContent>
            <ComboboxEmpty>No branch matches.</ComboboxEmpty>
            <ComboboxList>
              {(item) => {
                const b = (item as (typeof items)[number]).branch
                return (
                  <ComboboxItem key={b.name} value={b.name}>
                    <span className="flex w-full min-w-0 items-center gap-2">
                      <GitBranch className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
                      <span className="min-w-0 flex-1 truncate font-mono text-[13px]">{b.name}</span>
                      {b.default ? <span className="shrink-0 text-[11.5px] text-muted-foreground">default</span> : null}
                      {b.protected ? (
                        <ShieldCheck className="size-3.5 shrink-0 text-muted-foreground" aria-label="Protected" />
                      ) : null}
                    </span>
                  </ComboboxItem>
                )
              }}
            </ComboboxList>
          </ComboboxContent>
        </Combobox>
        {hidden}
        {!known && value ? (
          <FieldDescription>
            <code className="font-mono">{value}</code> no longer exists on GitHub. Pick a branch.
          </FieldDescription>
        ) : null}
      </Field>
    )
  }

  return (
    <Field>
      <FieldLabel htmlFor={id}>Branch</FieldLabel>
      <Input
        id={id}
        name={name}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder="Default branch"
        autoComplete="off"
        spellCheck={false}
        className="font-mono text-[13px]"
      />
      {repo && branches.isError ? (
        <FieldDescription>
          Couldn’t list branches ({branches.error instanceof ApiError ? branches.error.message : 'GitHub did not answer'}).
          Type one, or{' '}
          <button type="button" className="underline underline-offset-4" onClick={() => branches.refetch()}>
            try again
          </button>
          .
        </FieldDescription>
      ) : repo ? (
        <FieldDescription>This repository has no branches yet.</FieldDescription>
      ) : null}
    </Field>
  )
}

function Notice({
  title,
  children,
  action,
  tone,
}: {
  title: string
  children?: ReactNode
  action?: ReactNode
  tone?: 'fail'
}) {
  return (
    <Banner status={tone === 'fail' ? 'error' : 'default'} contrast={tone === 'fail' ? 'high' : 'low'}>
      <BannerTitle>{title}</BannerTitle>
      {children ? <BannerDescription>{children}</BannerDescription> : null}
      {action ? <div className="mt-2">{action}</div> : null}
    </Banner>
  )
}

function RetryButton({ onClick }: { onClick: () => void }) {
  return (
    <Button type="button" variant="outline" size="sm" onClick={onClick}>
      Try again
    </Button>
  )
}
