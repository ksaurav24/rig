import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { ExternalLink, GitBranch, Lock, Trash2 } from 'lucide-react'
import { useRef, useState, type FormEvent } from 'react'
import { toast } from 'sonner'
import { api, unwrap, type Schemas } from '@/api/client'
import { githubQuery, githubReposQuery, meQuery } from '@/api/queries'
import { CopyButton } from '@/components/app/copy-button'
import { Page, Panel } from '@/components/app/page'
import { BrandMark } from '@/components/app/service-icons'
import { EmptyState, ErrorState } from '@/components/app/state'
import { Banner, BannerDescription, BannerTitle } from '@/components/fluid/banner'
import { Field, FieldDescription, FieldGroup, FieldLabel } from '@/components/fluid/field'
import { Input } from '@/components/fluid/input'
import { InputCopy } from '@/components/fluid/input-copy'
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
      description="Clone private repositories and deploy automatically on push."
      className="max-w-4xl"
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
    <Panel title="Connect GitHub App" description="One GitHub App shared across all projects.">
      <div className="grid gap-6 md:grid-cols-[1fr_minmax(0,21rem)] md:gap-10">
        <ol className="grid gap-4 text-[13px]">
          {[
            ['Create the App', 'Registers a private GitHub App with read-only code and push webhook permissions.'],
            ['Install it', 'Pick which repositories or organizations the App can access.'],
            ['Deploy on push', 'New apps list those repos and redeploy automatically on push.'],
          ].map(([title, body], i) => (
            <li key={title} className="flex gap-3">
              <span className="grid size-6 shrink-0 place-items-center rounded-full bg-muted font-mono text-[12px] text-muted-foreground tabular-nums">
                {i + 1}
              </span>
              <span>
                <span className="block font-medium text-foreground">{title}</span>
                <span className="block text-muted-foreground">{body}</span>
              </span>
            </li>
          ))}
        </ol>
        {admin ? (
          <form onSubmit={submit} className="md:border-l md:border-border/80 md:pl-8">
            <FieldGroup>
              <Field>
                <FieldLabel htmlFor="gh-org">Organization (optional)</FieldLabel>
                <Input
                  id="gh-org"
                  value={organization}
                  onChange={(e) => setOrganization(e.target.value)}
                  placeholder="Personal account if empty"
                  autoComplete="off"
                />
                <FieldDescription>GitHub org that will own the App.</FieldDescription>
              </Field>
              <Button type="submit" loading={start.isPending}>
                <BrandMark name="github" data-icon="inline-start" />
                Create GitHub App
              </Button>
            </FieldGroup>
          </form>
        ) : (
          <p className="text-[13px] text-muted-foreground md:border-l md:border-border/80 md:pl-8">
            Only an admin can connect GitHub. Public repositories still work via Git URL.
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
        <p className="text-[13px] text-muted-foreground">
          Private repositories clone with short-lived installation tokens. Enable{' '}
          <span className="font-medium text-foreground">Deploy on push</span> in an app&apos;s Settings tab.
        </p>
      </Panel>

      {admin && (
        <Panel title="Repositories" flush>
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
            <EmptyState icon={GitBranch} title="No repositories installed">
              Install the App on the repositories you want to deploy.
            </EmptyState>
          ) : (
            <ul className="divide-y divide-border/80">
              {repos.data.map((r) => (
                <li key={r.full_name} className="flex min-w-0 items-center gap-3 px-4 py-2.5 text-[13px]">
                  <span className="min-w-0 flex-1 truncate font-medium text-foreground">{r.full_name}</span>
                  {r.private && <Lock className="size-3.5 shrink-0 text-muted-foreground" aria-label="Private" />}
                  <span className="hidden shrink-0 font-mono text-xs text-muted-foreground sm:inline">{r.default_branch}</span>
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
              Rig removes the App&apos;s keys. Private repos stop cloning and push deploys stop. Delete the App on GitHub too if no longer needed.
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
    <Banner status="info" contrast="low">
      <BannerTitle>Private network fallback</BannerTitle>
      <BannerDescription>
        {status.poll_interval > 0
          ? `Rig polls deploy-on-push branches every ${status.poll_interval}s when GitHub webhooks cannot reach this server.`
          : 'Branch polling is disabled on this server (--poll-interval 0).'}
      </BannerDescription>
      {status.app ? (
        <div className="mt-2.5 max-w-lg">
          <InputCopy value={webhook} size="compact" />
        </div>
      ) : null}
    </Banner>
  )
}
