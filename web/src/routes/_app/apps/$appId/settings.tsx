import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute, Link, useNavigate } from '@tanstack/react-router'
import { useState, type FormEvent } from 'react'

import { api, unwrap, type Schemas } from '@/api/client'
import { appQuery, githubQuery, githubReposQuery, meQuery } from '@/api/queries'
import { CopyButton } from '@/components/app/copy-button'
import { BranchField, GithubSource } from '@/components/app/new-app-dialog'
import { Panel } from '@rig/ui/app/page'
import { ErrorState } from '@/components/app/state'
import { Field, FieldDescription, FieldGroup, FieldLabel } from '@rig/ui/field'
import { Input } from '@rig/ui/input'
import { Switch } from '@rig/ui/switch'
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
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { keepComboboxEnter } from '@/lib/forms'
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
      <div className="max-w-3xl">
        <Panel title="General">
          <div className="space-y-3">
            {Array.from({ length: 4 }, (_, i) => (
              <Skeleton key={i} className="h-9" />
            ))}
          </div>
        </Panel>
      </div>
    )
  }
  if (app.isError) return <ErrorState error={app.error} onRetry={() => void app.refetch()} />

  const current = app.data
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
    <div className="flex max-w-3xl flex-col gap-6">
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
      <div className="flex items-center justify-between gap-4">
        <div className="space-y-0.5 text-[13px]">
          <div className="font-medium text-foreground">
            Auto-deploy commits on <code className="font-mono text-xs">{app.git_branch || 'default branch'}</code>
          </div>
          <div className="text-[12.5px] text-muted-foreground">
            Uses the{' '}
            <Link to="/settings/github" className="text-foreground underline underline-offset-4">
              GitHub App
            </Link>{' '}
            webhook, with branch polling fallback.
          </div>
        </div>
        <Switch
          checked={on}
          disabled={!canEdit || toggle.isPending}
          onToggle={() => toggle.mutate(!on)}
          aria-label="Deploy on push"
        />
      </div>
    </Panel>
  )
}

function GeneralPanel({ app, canEdit }: { app: App; canEdit: boolean }) {
  const queryClient = useQueryClient()
  const [dirty, setDirty] = useState(false)

  const save = useMutation({
    mutationFn: (body: Schemas['UpdateAppRequest']) =>
      unwrap(api.PATCH('/api/v1/apps/{id}', { params: { path: { id: app.id } }, body })),
    meta: { success: 'Settings saved' },
    onSuccess: () => {
      setDirty(false)
      return queryClient.invalidateQueries({ queryKey: ['apps', app.id] })
    },
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
      body.git_branch = text('git_branch')
    }
    save.mutate(body)
  }

  return (
    <Panel title="General" description="Applied on the next deploy.">
      <form onSubmit={submit} onChange={() => setDirty(true)} onKeyDown={keepComboboxEnter}>
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
                <SourceField app={app} onDirty={() => setDirty(true)} />
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
                <FieldLabel htmlFor="app-port">Container port</FieldLabel>
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
                <FieldDescription>Leave empty to check the open port.</FieldDescription>
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
                <FieldDescription>Defaults to 512 MB when empty.</FieldDescription>
              </Field>
              <Field>
                <FieldLabel htmlFor="app-cpu">CPU limit (cores)</FieldLabel>
                <Input
                  id="app-cpu"
                  name="cpu"
                  type="number"
                  inputMode="decimal"
                  min={0}
                  step="any"
                  defaultValue={app.cpu_millis ? app.cpu_millis / 1000 : ''}
                  placeholder="Unlimited"
                />
                <FieldDescription>E.g. 0.5 for half a core.</FieldDescription>
              </Field>
            </div>
          </FieldGroup>

          {canEdit ? (
            <div className="mt-5 flex items-center justify-end gap-3 border-t border-border/70 pt-4">
              {dirty ? <span className="text-[12.5px] text-muted-foreground">Unsaved changes</span> : null}
              <Button type="submit" disabled={!dirty || save.isPending}>
                {save.isPending ? <Spinner data-icon="inline-start" /> : null}
                Save changes
              </Button>
            </div>
          ) : (
            <p className="mt-5 text-[12.5px] text-muted-foreground">
              Developer role or higher required to edit settings.
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
    <Panel title="Danger zone" tone="danger">
      <div className="flex flex-wrap items-center justify-between gap-x-6 gap-y-3">
        <p className="max-w-[55ch] text-[13px] text-muted-foreground">
          Permanently deletes this app, its container, deployments, variables, and domains.
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
            The container, deployments, variables, and domains will be permanently removed.
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
      title="Deploy webhook"
      description={
        app.source_kind === 'git'
          ? `Trigger a deploy from GitHub, GitLab, or CI for ${app.git_branch ?? 'the default branch'}.`
          : 'Trigger a redeploy from CI after pushing a new image.'
      }
    >
      {hook && url ? (
        <dl className="grid gap-4 text-[13px]">
          <div className="grid gap-1.5">
            <dt className="text-[12.5px] font-medium text-muted-foreground">Payload URL</dt>
            <dd className="flex min-w-0 items-center gap-1 rounded-lg border border-border/80 bg-surface/60 py-1 pr-1 pl-3">
              <code className="min-w-0 flex-1 truncate font-mono text-xs">{url}</code>
              <CopyButton value={url} label="Copy URL" />
            </dd>
          </div>
          <div className="grid gap-1.5">
            <dt className="text-[12.5px] font-medium text-muted-foreground">Secret</dt>
            <dd className="flex min-w-0 items-center gap-1 rounded-lg border border-border/80 bg-surface/60 py-1 pr-1 pl-3">
              <code className="min-w-0 flex-1 truncate font-mono text-xs">{hook.secret}</code>
              <CopyButton value={hook.secret} label="Copy secret" />
            </dd>
          </div>
          <p className="text-[12.5px] text-muted-foreground">
            Send <code className="font-mono">application/json</code> signed with{' '}
            <code className="font-mono">X-Hub-Signature-256</code>, <code className="font-mono">X-Gitlab-Token</code>, or{' '}
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

function SourceField({ app, onDirty }: { app: App; onDirty: () => void }) {
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
            onDirty()
          }}
          onUseGitUrl={() => setTyped(true)}
        />
        <BranchField
          id="app-git-branch"
          name="git_branch"
          repo={match}
          github
          value={branch}
          onChange={(b) => {
            setBranch(b)
            onDirty()
          }}
        />
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
          onChange={(e) => {
            setUrl(e.target.value)
            onDirty()
          }}
          required
          placeholder="https://github.com/acme/web.git"
          autoCapitalize="none"
          autoComplete="off"
          spellCheck={false}
        />
      </Field>
      <BranchField
        id="app-git-branch"
        name="git_branch"
        repo={null}
        github={false}
        value={branch}
        onChange={(b) => {
          setBranch(b)
          onDirty()
        }}
      />
    </>
  )
}
