import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute, Link, useNavigate } from '@tanstack/react-router'
import { useState, type FormEvent } from 'react'

import { api, unwrap, type Schemas } from '@/api/client'
import { appQuery, githubQuery, githubReposQuery, meQuery } from '@/api/queries'
import { BranchField, GithubSource } from '@/components/app/new-app-dialog'
import { keepComboboxEnter } from '@/lib/forms'
import { CopyButton } from '@/components/app/copy-button'
import { Panel } from '@/components/app/page'
import { ErrorState } from '@/components/app/state'
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
import { Field, FieldDescription, FieldGroup, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { Switch } from '@/components/ui/switch'
import { hasRole } from '@/lib/nav'

type App = Schemas['App']

export const Route = createFileRoute('/_app/apps/$appId/settings')({
  staticData: { crumb: 'Settings' },
  component: AppSettings,
})

function AppSettings() {
  const { appId } = Route.useParams()
  const app = useQuery(appQuery(appId))
  const { data: me } = useQuery(meQuery)
  const canEdit = me ? hasRole(me.role, 'developer') : false
  const canDelete = me ? hasRole(me.role, 'admin') : false

  if (app.isPending) {
    return (
      <Panel title="General">
        <div className="space-y-3">
          {Array.from({ length: 4 }, (_, i) => (
            <Skeleton key={i} className="h-9" />
          ))}
        </div>
      </Panel>
    )
  }
  if (app.isError) return <ErrorState error={app.error} onRetry={() => void app.refetch()} />

  const current = app.data
  // Re-key on the saved values so the form picks up changes made elsewhere
  // without wiping what someone is typing during a background refresh.
  const formKey = JSON.stringify([
    current.name,
    current.image,
    current.git_url,
    current.git_branch,
    current.dockerfile_path,
    current.port,
    current.health_path,
    current.memory_mb,
    current.cpu_millis,
  ])

  return (
    <div className="flex flex-col gap-6">
      <GeneralPanel key={formKey} app={current} canEdit={canEdit} />
      {current.source_kind === 'git' && <PushPanel app={current} canEdit={canEdit} />}
      {canEdit ? <WebhookPanel app={current} /> : null}
      {canDelete ? <DeletePanel app={current} /> : null}
    </div>
  )
}

function PushPanel({ app, canEdit }: { app: App; canEdit: boolean }) {
  const queryClient = useQueryClient()
  const toggle = useMutation({
    mutationFn: (auto_deploy: boolean) =>
      unwrap(api.PATCH('/api/v1/apps/{id}', { params: { path: { id: app.id } }, body: { auto_deploy } })),
    meta: { success: 'Saved' },
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['apps', app.id] }),
  })
  const on = toggle.isPending ? Boolean(toggle.variables) : app.auto_deploy

  return (
    <Panel title="Deploy on push">
      <label className="flex items-start justify-between gap-4">
        <span className="space-y-1 text-sm">
          <span className="block font-medium">Deploy every new commit on {app.git_branch || 'the default branch'}</span>
          <span className="block text-muted-foreground">
            Uses the <Link to="/settings/github" className="underline underline-offset-2">GitHub App</Link> push
            webhook, and branch polling when GitHub can't reach this server.
          </span>
        </span>
        <Switch
          checked={on}
          disabled={!canEdit || toggle.isPending}
          onCheckedChange={(v) => toggle.mutate(v)}
          aria-label="Deploy on push"
        />
      </label>
    </Panel>
  )
}

function GeneralPanel({ app, canEdit }: { app: App; canEdit: boolean }) {
  const queryClient = useQueryClient()

  const save = useMutation({
    mutationFn: (body: Schemas['UpdateAppRequest']) =>
      unwrap(api.PATCH('/api/v1/apps/{id}', { params: { path: { id: app.id } }, body })),
    meta: { success: 'Settings saved' },
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['apps', app.id] }),
  })

  function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault()
    if (!canEdit) return
    const form = new FormData(e.currentTarget)
    const text = (name: string) => String(form.get(name) ?? '').trim()

    const body: Schemas['UpdateAppRequest'] = {
      name: text('name'),
      port: Number(text('port')),
      health_path: text('health_path'),
      memory_mb: Math.round(Number(text('memory_mb') || 0)),
      cpu_millis: Math.round(Number(text('cpu') || 0) * 1000),
    }
    if (app.source_kind === 'image') {
      body.image = text('image')
    } else {
      body.git_url = text('git_url')
      body.dockerfile_path = text('dockerfile_path')
      // An empty branch clears it, which means the repository's default.
      body.git_branch = text('git_branch')
    }
    save.mutate(body)
  }

  return (
    <Panel title="General" description="How Rig builds and runs this app.">
      <form onSubmit={submit} onKeyDown={keepComboboxEnter}>
        <fieldset disabled={!canEdit} className="min-w-0">
          <FieldGroup>
            <Field>
              <FieldLabel htmlFor="app-name">Name</FieldLabel>
              <Input id="app-name" name="name" defaultValue={app.name} required maxLength={64} autoComplete="off" />
            </Field>

            {app.source_kind === 'image' ? (
              <Field>
                <FieldLabel htmlFor="app-image">Image</FieldLabel>
                <Input
                  id="app-image"
                  name="image"
                  className="font-mono"
                  defaultValue={app.image ?? ''}
                  required
                  placeholder="ghcr.io/acme/web:latest"
                  autoCapitalize="none"
                  autoComplete="off"
                  spellCheck={false}
                />
              </Field>
            ) : (
              <>
                <SourceField app={app} />
                <div className="grid gap-5 sm:grid-cols-2">
                  <Field>
                    <FieldLabel htmlFor="app-dockerfile">Dockerfile path</FieldLabel>
                    <Input
                      id="app-dockerfile"
                      name="dockerfile_path"
                      className="font-mono"
                      defaultValue={app.dockerfile_path}
                      required
                      placeholder="Dockerfile"
                      autoCapitalize="none"
                      autoComplete="off"
                      spellCheck={false}
                    />
                  </Field>
                </div>
              </>
            )}

            <div className="grid gap-5 sm:grid-cols-2">
              <Field>
                <FieldLabel htmlFor="app-port">Port</FieldLabel>
                <Input
                  id="app-port"
                  name="port"
                  type="number"
                  inputMode="numeric"
                  min={1}
                  max={65535}
                  step={1}
                  defaultValue={app.port}
                  required
                />
                <FieldDescription>The port the app listens on inside its container.</FieldDescription>
              </Field>
              <Field>
                <FieldLabel htmlFor="app-health">Health check path</FieldLabel>
                <Input
                  id="app-health"
                  name="health_path"
                  className="font-mono"
                  defaultValue={app.health_path ?? ''}
                  placeholder="/healthz"
                  autoCapitalize="none"
                  autoComplete="off"
                  spellCheck={false}
                />
                <FieldDescription>
                  A deploy goes live once this path answers. Leave empty to check the port instead.
                </FieldDescription>
              </Field>
            </div>

            <div className="grid gap-5 sm:grid-cols-2">
              <Field>
                <FieldLabel htmlFor="app-memory">Memory limit (MB)</FieldLabel>
                <Input
                  id="app-memory"
                  name="memory_mb"
                  type="number"
                  inputMode="numeric"
                  min={0}
                  step={1}
                  defaultValue={app.memory_mb ?? ''}
                  placeholder="512"
                />
                <FieldDescription>Leave empty for the 512 MB default. Changes apply on the next deploy.</FieldDescription>
              </Field>
              <Field>
                <FieldLabel htmlFor="app-cpu">CPU limit (CPUs)</FieldLabel>
                <Input
                  id="app-cpu"
                  name="cpu"
                  type="number"
                  inputMode="decimal"
                  min={0}
                  step="any"
                  defaultValue={app.cpu_millis ? app.cpu_millis / 1000 : ''}
                  placeholder="No limit"
                />
                <FieldDescription>For example 0.5 for half a core. Leave empty for no limit.</FieldDescription>
              </Field>
            </div>
          </FieldGroup>

          {canEdit ? (
            <div className="mt-5 flex flex-wrap items-center gap-x-4 gap-y-2">
              <Button type="submit" disabled={save.isPending}>
                {save.isPending ? <Spinner data-icon="inline-start" /> : null}
                Save changes
              </Button>
              <p className="text-[0.8125rem] text-muted-foreground">Saved settings apply on the next deploy.</p>
            </div>
          ) : (
            <p className="mt-5 text-[0.8125rem] text-muted-foreground">
              You can view these settings. Changing them needs the developer role.
            </p>
          )}
        </fieldset>
      </form>
    </Panel>
  )
}

function DeletePanel({ app }: { app: App }) {
  const [open, setOpen] = useState(false)

  return (
    <Panel title="Delete app">
      <div className="flex flex-wrap items-center justify-between gap-x-6 gap-y-3">
        <p className="max-w-[60ch] text-muted-foreground">
          Removes the container, its deployments, variables and domains. This cannot be undone.
        </p>
        <Button variant="destructive" onClick={() => setOpen(true)}>
          Delete app
        </Button>
      </div>
      <DeleteAppDialog app={app} open={open} onOpenChange={setOpen} />
    </Panel>
  )
}

function DeleteAppDialog({
  app,
  open,
  onOpenChange,
}: {
  app: App
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const [typed, setTyped] = useState('')

  const close = (next: boolean) => {
    if (!next) setTyped('')
    onOpenChange(next)
  }

  const remove = useMutation({
    mutationFn: () => unwrap(api.DELETE('/api/v1/apps/{id}', { params: { path: { id: app.id } } })),
    meta: { success: 'App deleted' },
    onSuccess: async () => {
      // Drop the gone app from the cached list first, so the project page
      // never flashes it; then drop its own data instead of refetching a 404.
      queryClient.setQueryData<Schemas['App'][]>(['apps'], (list) => list?.filter((a) => a.id !== app.id))
      void queryClient.invalidateQueries({ queryKey: ['projects'] })
      await navigate({ to: '/projects/$projectId', params: { projectId: app.project_id } })
      queryClient.removeQueries({ queryKey: ['apps', app.id] })
      void queryClient.invalidateQueries({ queryKey: ['apps'], exact: true })
    },
  })

  return (
    <AlertDialog open={open} onOpenChange={close}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Delete “{app.name}”?</AlertDialogTitle>
          <AlertDialogDescription>
            The container, its deployments, variables and domains are removed from the server. This cannot be undone.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <Field>
          <FieldLabel htmlFor="app-delete-confirm">Type {app.name} to confirm</FieldLabel>
          <Input
            id="app-delete-confirm"
            value={typed}
            onChange={(e) => setTyped(e.target.value)}
            autoComplete="off"
            spellCheck={false}
          />
        </Field>
        <AlertDialogFooter>
          <AlertDialogCancel>Keep app</AlertDialogCancel>
          <AlertDialogAction
            variant="destructive"
            disabled={remove.isPending || typed !== app.name}
            onClick={() => remove.mutate()}
          >
            Delete app
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}

/** Push-to-deploy. The secret stays hidden until someone asks for it, and
 *  each reveal is audited on the server. */
function WebhookPanel({ app }: { app: Schemas['App'] }) {
  const [hook, setHook] = useState<Schemas['AppWebhook'] | null>(null)
  const reveal = useMutation({
    mutationFn: () => unwrap(api.GET('/api/v1/apps/{id}/webhook', { params: { path: { id: app.id } } })),
    onSuccess: setHook,
  })
  const rotate = useMutation({
    mutationFn: () => unwrap(api.POST('/api/v1/apps/{id}/webhook/rotate', { params: { path: { id: app.id } } })),
    onSuccess: setHook,
    meta: { success: 'New webhook secret created. Update it wherever the old one is used' },
  })
  const url = hook ? `${window.location.origin}${hook.path}` : null
  const busy = reveal.isPending || rotate.isPending

  return (
    <Panel
      title="Deploy on push"
      description={
        app.source_kind === 'git'
          ? `Point a GitHub or GitLab webhook here and every push to ${app.git_branch ?? 'the default branch'} deploys.`
          : 'Call this webhook from CI after pushing a new image and the app redeploys.'
      }
    >
      {hook && url ? (
        <dl className="grid gap-4 text-sm">
          <div className="grid gap-1.5">
            <dt className="text-muted-foreground">Payload URL</dt>
            <dd className="flex min-w-0 items-center gap-2">
              <code className="min-w-0 flex-1 truncate rounded-md bg-muted px-2 py-1.5 font-mono text-xs">{url}</code>
              <CopyButton value={url} label="Copy URL" />
            </dd>
          </div>
          <div className="grid gap-1.5">
            <dt className="text-muted-foreground">Secret</dt>
            <dd className="flex min-w-0 items-center gap-2">
              <code className="min-w-0 flex-1 truncate rounded-md bg-muted px-2 py-1.5 font-mono text-xs">{hook.secret}</code>
              <CopyButton value={hook.secret} label="Copy secret" />
            </dd>
          </div>
          <p className="text-muted-foreground">
            Use content type <code className="font-mono">application/json</code>. Rig checks the{' '}
            <code className="font-mono">X-Hub-Signature-256</code> signature, or GitLab&apos;s{' '}
            <code className="font-mono">X-Gitlab-Token</code>. From CI, sign the body with HMAC-SHA256 and send it as{' '}
            <code className="font-mono">X-Rig-Signature: sha256=&lt;hex&gt;</code>.
          </p>
          <div>
            <Button variant="outline" size="sm" disabled={busy} onClick={() => rotate.mutate()}>
              {rotate.isPending ? <Spinner /> : null}
              Rotate secret
            </Button>
          </div>
        </dl>
      ) : (
        <Button variant="outline" size="sm" disabled={busy} onClick={() => reveal.mutate()}>
          {reveal.isPending ? <Spinner /> : null}
          Show webhook
        </Button>
      )}
    </Panel>
  )
}

/** The repository and branch: picked from the GitHub App when one is connected
 *  and the current URL is one of its repositories, typed otherwise. Picking
 *  another repository moves the branch to that repository's default. */
function SourceField({ app }: { app: App }) {
  const status = useQuery(githubQuery)
  const repos = useQuery({ ...githubReposQuery, enabled: Boolean(status.data?.app) })
  const [url, setUrl] = useState(app.git_url ?? '')
  const [branch, setBranch] = useState(app.git_branch ?? '')
  const [typed, setTyped] = useState(false)
  const norm = (u: string) => u.trim().replace(/\.git$/, '').toLowerCase()
  const match = repos.data?.find((r) => norm(r.clone_url) === norm(url)) ?? null
  const picker = !typed && Boolean(status.data?.app) && repos.isSuccess && (match !== null || !url.trim())

  if (picker) {
    return (
      <>
        <input type="hidden" name="git_url" value={url} />
        <GithubSource
          repo={match}
          onRepo={(r) => {
            setUrl(r?.clone_url ?? '')
            setBranch(r?.default_branch ?? '')
          }}
          onUseGitUrl={() => setTyped(true)}
        />
        <BranchField id="app-git-branch" name="git_branch" repo={match} github value={branch} onChange={setBranch} />
      </>
    )
  }
  return (
    <>
      <Field>
        <FieldLabel htmlFor="app-git-url">Git URL</FieldLabel>
        <Input
          id="app-git-url"
          name="git_url"
          className="font-mono"
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          required
          placeholder="https://github.com/acme/web.git"
          autoCapitalize="none"
          autoComplete="off"
          spellCheck={false}
        />
      </Field>
      <BranchField id="app-git-branch" name="git_branch" repo={null} github={false} value={branch} onChange={setBranch} />
    </>
  )
}
