import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Link, useNavigate } from '@tanstack/react-router'
import { ArrowLeft, ExternalLink, GitBranch, Globe, Lock, RefreshCw, ShieldCheck } from 'lucide-react'
import { useMemo, useState, type FormEvent, type ReactNode } from 'react'
import { toast } from 'sonner'
import { api, ApiError, unwrap, type Schemas } from '@/api/client'
import { githubBranchesQuery, githubQuery, githubReposQuery, meQuery } from '@/api/queries'
import { BrandMark } from '@/components/app/service-icons'
import { keepComboboxEnter } from '@/lib/forms'
import { Combobox, ComboboxContent, ComboboxEmpty, ComboboxInput, ComboboxItem, ComboboxList } from '@/components/fluid/combobox'
import { Button } from '@/components/ui/button'
import { Switch } from '@/components/ui/switch'
import { Checkbox } from '@/components/ui/checkbox'
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
import { Skeleton } from '@/components/ui/skeleton'
import { hasRole } from '@/lib/nav'
import { cn } from '@/lib/utils'

type Provider = 'github' | 'git' | 'image'
type Build = 'auto' | 'dockerfile'
type Repo = Schemas['GithubRepo']

const DEFAULT_DOCKERFILE = 'Dockerfile'

const PROVIDERS: { id: Provider; mark: 'github' | 'git' | 'docker'; title: string; hint: string }[] = [
  { id: 'github', mark: 'github', title: 'GitHub', hint: 'Pick a repository; deploy on every push' },
  { id: 'git', mark: 'git', title: 'Git URL', hint: 'Any public https:// or ssh:// repo' },
  { id: 'image', mark: 'docker', title: 'Docker image', hint: 'Pull a prebuilt image from a registry' },
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

/**
 * Creates an app in a project, then (unless unticked) starts its first
 * deployment. Two steps, like the source picker on most PaaS dashboards:
 * choose where the code comes from, then fill in that source and the run
 * settings on one screen.
 */
export function NewAppDialog({ project, onClose }: { project: Schemas['Project'] | null; onClose: () => void }) {
  return (
    <Dialog open={project !== null} onOpenChange={(open) => (open ? undefined : onClose())}>
      <DialogContent className="gap-5 sm:max-w-xl">
        {/* Remount per opening so every visit starts at the provider step. */}
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
  // Only GitHub sources get push events; plain Git URLs fall back to polling,
  // which works too but is opt-in, as on the app's settings page.
  const [autoDeploy, setAutoDeploy] = useState(true)
  // Without a connected App there is nothing to fill in below the source,
  // so the form collapses to the notice and its way out.
  const github = useQuery({ ...githubQuery, enabled: provider === 'github' })
  const githubMissing = provider === 'github' && github.isSuccess && !github.data.app

  const create = useMutation({
    mutationFn: async (body: Schemas['CreateAppRequest']) => {
      let app = await unwrap(api.POST('/api/v1/apps', { body }))
      if (provider === 'github' && autoDeploy) {
        // Create has no flag for this, so set it straight after. A failure
        // leaves it off, which the app's settings page shows and can retry.
        app = await unwrap(
          api.PATCH('/api/v1/apps/{id}', { params: { path: { id: app.id } }, body: { auto_deploy: true } }),
        ).catch(() => app)
      }
      if (!deployNow) return { app, deployed: null }
      // The app exists from here on, so a failed first deploy must not look
      // like a failed create: land on the app either way.
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
          <DialogDescription>In {project.name}. Where does the code come from?</DialogDescription>
        </DialogHeader>
        <div className="grid gap-2 sm:grid-cols-3">
          {PROVIDERS.map((p) => (
            <button
              key={p.id}
              type="button"
              onClick={() => setProvider(p.id)}
              className={cn(
                'group flex flex-col items-start gap-3 rounded-lg p-4 text-left outline-none',
                'shadow-[0_0_0_1px_var(--border)] transition-[box-shadow,background-color] duration-150',
                'hover:bg-hover hover:shadow-[0_0_0_1px_var(--ring)] focus-visible:ring-1 focus-visible:ring-[color:var(--focus-ring)]',
              )}
            >
              <BrandMark name={p.mark} className="size-5" />
              <span>
                <span className="block font-medium">{p.title}</span>
                <span className="mt-0.5 block text-[13px] leading-snug text-muted-foreground">{p.hint}</span>
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
        <DialogDescription>In {project.name}. You can change all of this later in the app’s settings.</DialogDescription>
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
              <FieldDescription>Public repositories only. For private GitHub repos, use the GitHub source.</FieldDescription>
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
              <FieldDescription>Pulled from its registry on every deploy.</FieldDescription>
            </Field>
          )}
          {isGit && !githubMissing ? (
            <BranchField id="app-git-branch" repo={provider === 'github' ? repo : null} github={provider === 'github'} value={branch} onChange={setBranch} />
          ) : null}
          {provider === 'github' && !githubMissing ? (
            <label className="flex items-start justify-between gap-4 text-[13px]">
              <span>
                <span className="block font-medium">Deploy on push</span>
                <span className="block text-muted-foreground">
                  Every push to {branch.trim() ? <code className="font-mono">{branch.trim()}</code> : 'the branch'}{' '}
                  deploys a new version.
                </span>
              </span>
              <Switch checked={autoDeploy} onCheckedChange={setAutoDeploy} aria-label="Deploy on push" />
            </label>
          ) : null}
        </Section>

        {githubMissing ? null : (
        <>
        {isGit ? (
          <Section title="Build">
            <div role="radiogroup" aria-label="Build type" className="grid gap-2 sm:grid-cols-2">
              <Choice selected={build === 'auto'} onSelect={() => setBuild('auto')} title="Auto-detect">
                Uses the Dockerfile at the root if there is one, otherwise Railpack works out the build.
              </Choice>
              <Choice selected={build === 'dockerfile'} onSelect={() => setBuild('dockerfile')} title="Dockerfile">
                Build from a Dockerfile somewhere else in the repository.
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
                <FieldDescription>Relative to the repository root. The build context is the root.</FieldDescription>
              </Field>
            ) : null}
          </Section>
        ) : null}

        <Section title="Run">
          <div className="grid gap-4 sm:grid-cols-[1fr_8rem]">
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
          <FieldDescription className="-mt-2">
            The port the app listens on inside its container. Rig also sets it as <code className="font-mono">PORT</code>.
          </FieldDescription>
          <details className="group rounded-lg shadow-[0_0_0_1px_var(--border)] [&[open]>summary]:border-b">
            <summary className="flex cursor-pointer list-none items-center justify-between px-3 py-2 text-[13px] font-medium select-none marker:hidden">
              Advanced
              <span className="text-muted-foreground group-open:hidden">Health check, persistent paths</span>
            </summary>
            <div className="grid gap-4 p-3">
              <Field>
                <FieldLabel htmlFor="app-health">Health check path</FieldLabel>
                <Input id="app-health" name="health_path" placeholder="/healthz" autoComplete="off" spellCheck={false} className="font-mono text-[13px]" />
                <FieldDescription>Optional. Rig waits for a 2xx here before sending traffic to a new deploy.</FieldDescription>
              </Field>
              <Field>
                <FieldLabel htmlFor="app-volumes">Persistent paths</FieldLabel>
                <Input id="app-volumes" name="volumes" placeholder="/data" autoComplete="off" spellCheck={false} className="font-mono text-[13px]" />
                <FieldDescription>
                  Folders that keep their files across deploys, separated by commas. Apps with persistent paths restart in
                  place instead of swapping.
                </FieldDescription>
              </Field>
            </div>
          </details>
        </Section>
        </>
        )}
      </FieldGroup>

      <DialogFooter className="items-center sm:justify-between">
        {githubMissing ? <span /> : (
        <label className="flex items-center gap-2 text-[13px] text-muted-foreground select-none">
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
    <fieldset className="grid gap-4">
      <legend className="mb-3 text-[11px] font-medium tracking-[0.06em] text-muted-foreground uppercase">{title}</legend>
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
        <span className="mt-0.5 block text-[12.5px] leading-snug text-muted-foreground">{children}</span>
      </span>
    </button>
  )
}

/**
 * The connected GitHub App and the repositories it can see. Every state a
 * user can hit is handled here: no App, App on no repos, list failing,
 * still loading, and a repo that is simply not in the list.
 */
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
          ? 'Connect a GitHub App once for the organization, then pick repositories here and deploy on every push.'
          : 'An admin has to connect GitHub for the organization first. Public repositories work through a Git URL meanwhile.'}
      </Notice>
    )
  }

  const account = (
    <div className="flex items-center gap-2.5 rounded-lg px-3 py-2 shadow-[0_0_0_1px_var(--border)]">
      <BrandMark name="github" className="size-4 shrink-0" />
      <div className="min-w-0 flex-1 leading-tight">
        <div className="truncate text-[13px] font-medium">{app.owner}</div>
        <div className="truncate text-[12px] text-muted-foreground">via the {app.name} app</div>
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
        title="The app can’t see any repositories yet"
        action={
          <a href={app.install_url} target="_blank" rel="noreferrer" className="font-medium text-foreground underline-offset-4 hover:underline">
            Choose repositories on GitHub
          </a>
        }
      >
        Install it on your account or organization and grant it the repos to deploy, then come back. The list refreshes
        when this window regains focus.
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
                  {/* One row: the Fluid item stacks direct children. */}
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

/**
 * The branch to deploy. For a GitHub repository it lists the real branches;
 * otherwise (or when GitHub can't list them) it is typed by hand.
 */
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
  /** The GitHub source is selected, so a repository has to be picked first. */
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
            <code className="font-mono">{value}</code> no longer exists on GitHub. Pick a branch to deploy.
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
        <FieldDescription>The repository has no branches yet. Push one, then refresh.</FieldDescription>
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
    <div
      className={cn(
        'grid gap-1 rounded-lg px-3.5 py-3 text-[13px] shadow-[0_0_0_1px_var(--border)]',
        tone === 'fail' ? 'bg-fail/8' : 'bg-muted/50',
      )}
    >
      <div className="font-medium">{title}</div>
      {children ? <div className="leading-snug text-muted-foreground">{children}</div> : null}
      {action ? <div className="mt-1">{action}</div> : null}
    </div>
  )
}

function RetryButton({ onClick }: { onClick: () => void }) {
  return (
    <Button type="button" variant="outline" size="sm" onClick={onClick}>
      Try again
    </Button>
  )
}
