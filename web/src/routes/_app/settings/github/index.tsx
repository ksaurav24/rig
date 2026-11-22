import { BrandMark } from '@/components/app/service-icons'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { ExternalLink, GitBranch, Lock, Trash2 } from 'lucide-react'
import { useRef, useState, type FormEvent } from 'react'
import { toast } from 'sonner'
import { api, unwrap, type Schemas } from '@/api/client'
import { githubQuery, githubReposQuery, meQuery } from '@/api/queries'
import { CopyButton } from '@/components/app/copy-button'
import { Page, Panel } from '@/components/app/page'
import { EmptyState, ErrorState } from '@/components/app/state'
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
import { timeAgo } from '@/lib/format'
import { hasRole } from '@/lib/nav'

export const Route = createFileRoute('/_app/settings/github/')({
  staticData: { crumb: 'GitHub' },
  loader: ({ context }) => void context.queryClient.prefetchQuery(githubQuery),
  component: GithubPage,
})

function GithubPage() {
  const status = useQuery(githubQuery)
  const me = useQuery(meQuery)
  const admin = me.data ? hasRole(me.data.role, 'admin') : false

  return (
    <Page
      title="GitHub"
      description="Connect a GitHub App so Rig can clone private repositories and deploy on every push."
    >
      {status.isPending ? (
        <Panel>
          <Skeleton className="h-24" />
        </Panel>
      ) : status.isError ? (
        <ErrorState error={status.error} onRetry={() => status.refetch()} />
      ) : status.data.app ? (
        <Connected status={status.data} admin={admin} />
      ) : (
        <Connect admin={admin} />
      )}
      {status.data && <Polling status={status.data} />}
    </Page>
  )
}

type GithubStatus = Schemas['GithubStatus']

function Connect({ admin }: { admin: boolean }) {
  const [organization, setOrganization] = useState('')
  const form = useRef<HTMLFormElement>(null)
  const manifestInput = useRef<HTMLInputElement>(null)

  const start = useMutation({
    mutationFn: () =>
      unwrap(
        api.POST('/api/v1/github/manifest', {
          body: { organization: organization.trim() || null },
        }),
      ),
    onSuccess: (m) => {
      // GitHub's manifest flow is a plain form POST from the browser.
      if (!form.current || !manifestInput.current) return
      form.current.action = m.post_url
      manifestInput.current.value = m.manifest
      form.current.submit()
    },
  })

  const submit = (e: FormEvent) => {
    e.preventDefault()
    start.mutate()
  }

  return (
    <Panel title="Connect GitHub" description="One App for the organization. Every project can then pick its repositories.">
      <div className="grid gap-6 md:grid-cols-[1fr_minmax(0,22rem)] md:gap-10">
        <ol className="grid gap-4 text-[13px]">
          {[
            ['Create the App', 'Rig registers a private GitHub App for you. It asks only to read code and receive push events.'],
            ['Install it', 'Choose which repositories or organizations the App can see. You can change this at any time.'],
            ['Pick a repository', 'New apps list those repositories, and every push to the chosen branch redeploys.'],
          ].map(([title, body], i) => (
            <li key={title} className="flex gap-3">
              <span className="grid size-6 shrink-0 place-items-center rounded-full bg-muted font-mono text-[12px] text-muted-foreground tabular-nums">
                {i + 1}
              </span>
              <span>
                <span className="block font-medium">{title}</span>
                <span className="block text-muted-foreground">{body}</span>
              </span>
            </li>
          ))}
        </ol>
        {admin ? (
          <form onSubmit={submit} className="md:border-l md:pl-10">
            <FieldGroup>
              <Field>
                <FieldLabel htmlFor="gh-org">Organization (optional)</FieldLabel>
                <Input
                  id="gh-org"
                  value={organization}
                  onChange={(e) => setOrganization(e.target.value)}
                  placeholder="Leave empty for your own account"
                  autoComplete="off"
                />
                <FieldDescription>The GitHub organization that will own the App.</FieldDescription>
              </Field>
              <Button type="submit" loading={start.isPending}>
                <BrandMark name="github" data-icon="inline-start" />
                Create GitHub App
              </Button>
            </FieldGroup>
          </form>
        ) : (
          <p className="text-[13px] text-muted-foreground md:border-l md:pl-10">
            Only an admin can connect GitHub. Until then, public repositories deploy through a Git URL.
          </p>
        )}
      </div>
      <form ref={form} method="post" className="hidden" aria-hidden>
        <input ref={manifestInput} type="hidden" name="manifest" />
      </form>
    </Panel>
  )
}

function Connected({ status, admin }: { status: GithubStatus; admin: boolean }) {
  const app = status.app!
  const qc = useQueryClient()
  const [confirm, setConfirm] = useState(false)
  const repos = useQuery({ ...githubReposQuery, enabled: admin })

  const disconnect = useMutation({
    mutationFn: () => unwrap(api.DELETE('/api/v1/github')),
    onSuccess: () => {
      setConfirm(false)
      toast.success('GitHub disconnected', {
        description: 'Delete the App on GitHub too if you no longer need it.',
      })
      void qc.invalidateQueries({ queryKey: ['github'] })
    },
  })

  return (
    <>
      <Panel
        title={app.name}
        description={`Owned by ${app.owner} · connected ${timeAgo(app.created_at)}`}
        actions={
          <div className="flex flex-wrap gap-2">
            <Button variant="outline" nativeButton={false} render={<a href={app.install_url} target="_blank" rel="noreferrer" />}>
              Install on repositories
              <ExternalLink data-icon="inline-end" />
            </Button>
            {admin && (
              <Button variant="ghost" onClick={() => setConfirm(true)} aria-label="Disconnect GitHub">
                <Trash2 data-icon="inline-start" />
                Disconnect
              </Button>
            )}
          </div>
        }
      >
        <p className="text-sm text-muted-foreground">
          Apps built from a repository this App is installed on clone with a short-lived token. Turn on{' '}
          <span className="font-medium text-foreground">Deploy on push</span> in an app's settings to deploy
          every new commit on its branch.
        </p>
      </Panel>

      {admin && (
        <Panel title="Repositories" description="What the App can read right now." flush>
          {repos.isPending ? (
            <div className="space-y-2 p-4">
              <Skeleton className="h-8" />
              <Skeleton className="h-8" />
            </div>
          ) : repos.isError ? (
            <div className="p-4">
              <ErrorState error={repos.error} onRetry={() => repos.refetch()} />
            </div>
          ) : repos.data.length === 0 ? (
            <EmptyState
              icon={GitBranch}
              title="Not installed anywhere yet"
            >
              Install the App on the repositories you want to deploy.
            </EmptyState>
          ) : (
            <ul className="divide-y">
              {repos.data.map((r) => (
                <li key={r.full_name} className="flex min-w-0 items-center gap-3 px-4 py-2.5 text-sm">
                  <span className="min-w-0 flex-1 truncate font-medium">{r.full_name}</span>
                  {r.private && <Lock className="size-3.5 shrink-0 text-muted-foreground" aria-label="Private" />}
                  <span className="hidden shrink-0 text-muted-foreground sm:inline">{r.default_branch}</span>
                  <CopyButton value={r.clone_url} label="Copy clone URL" />
                </li>
              ))}
            </ul>
          )}
        </Panel>
      )}

      <AlertDialog open={confirm} onOpenChange={setConfirm}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Disconnect GitHub?</AlertDialogTitle>
            <AlertDialogDescription>
              Rig forgets the App's keys. Private repositories stop cloning and pushes stop deploying. The App
              stays on GitHub until you delete it there.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={() => disconnect.mutate()} disabled={disconnect.isPending}>
              {disconnect.isPending && <Spinner data-icon="inline-start" />}
              Disconnect
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}

function Polling({ status }: { status: GithubStatus }) {
  const webhook = typeof window === 'undefined' ? status.webhook_url : window.location.origin + status.webhook_url
  return (
    <Panel title="When GitHub can't reach this server">
      <div className="space-y-3 text-sm text-muted-foreground">
        <p>
          {status.poll_interval > 0
            ? `Rig also checks each deploy-on-push branch every ${status.poll_interval} seconds, so servers on a private network still deploy new commits.`
            : 'Branch polling is off on this server (--poll-interval 0).'}
        </p>
        {status.app && (
          <div className="flex min-w-0 items-center gap-2">
            <span className="shrink-0">Webhook</span>
            <code className="min-w-0 flex-1 truncate rounded bg-muted px-2 py-1 font-mono text-xs text-foreground">
              {webhook}
            </code>
            <CopyButton value={webhook} label="Copy webhook URL" />
          </div>
        )}
      </div>
    </Panel>
  )
}
