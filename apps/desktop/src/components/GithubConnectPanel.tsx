import { ExternalLink, GitBranch, Loader2, X } from 'lucide-react'
import { useState, type FormEvent } from 'react'
import { toast } from 'sonner'
import { Banner, BannerDescription, BannerTitle } from '@rig/ui/banner'
import { Button } from '@rig/ui/button'
import { Field, FieldDescription, FieldLabel } from '@rig/ui/field'
import { Input } from '@rig/ui/input'
import { desktopApi } from '@/lib/ipc'
import type { GithubStatus } from '@/lib/types'

interface GithubConnectPanelProps {
  serverId: string
  compact?: boolean
  onConnected: (status: GithubStatus) => void
  onUseGitUrl?: () => void
}

export function GithubConnectPanel({
  serverId,
  compact = false,
  onConnected,
  onUseGitUrl,
}: GithubConnectPanelProps) {
  const [organization, setOrganization] = useState('')
  const [connecting, setConnecting] = useState(false)
  const [error, setError] = useState<string | null>(null)

  async function handleConnect(e: FormEvent) {
    e.preventDefault()
    setError(null)
    setConnecting(true)
    try {
      const status = await desktopApi.connectGithubApp(serverId, organization)
      toast.success('GitHub App connected', {
        description: status.app
          ? `Connected ${status.app.name} (${status.app.owner}). Install it on your repositories to deploy.`
          : undefined,
      })
      onConnected(status)
    } catch (err) {
      const msg = String(err)
      if (!msg.toLowerCase().includes('cancelled')) {
        setError(msg)
      }
    } finally {
      setConnecting(false)
    }
  }

  async function handleCancel() {
    await desktopApi.cancelGithubConnect()
    setConnecting(false)
  }

  if (compact) {
    return (
      <div className="space-y-3.5 rounded-xl border border-border/80 bg-surface/60 p-4">
        <div className="flex items-start justify-between gap-3">
          <div>
            <div className="flex items-center gap-2 text-[14px] font-medium text-foreground">
              <GitBranch className="size-4 text-primary" />
              Connect a GitHub account to this server
            </div>
            <p className="mt-1 text-[12.5px] leading-relaxed text-muted-foreground">
              Creates a private GitHub App owned by your account or organization so you can pick
              private repositories, list branches, and deploy automatically on push.
            </p>
          </div>
        </div>

        {error && (
          <Banner role="alert" status="error" contrast="high" size="compact">
            <BannerDescription>{error}</BannerDescription>
          </Banner>
        )}

        {connecting ? (
          <div className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-primary/30 bg-primary/8 px-3.5 py-3">
            <div className="flex items-center gap-2.5 text-[13px] text-foreground">
              <Loader2 className="size-4 shrink-0 animate-spin text-primary" />
              <span>Finish creating the GitHub App in your browser…</span>
            </div>
            <Button
              type="button"
              variant="ghost"
              size="compact"
              leadingIcon={X}
              onClick={() => void handleCancel()}
            >
              Cancel
            </Button>
          </div>
        ) : (
          <div className="flex flex-wrap items-end gap-2.5">
            <Field className="min-w-[200px] flex-1">
              <FieldLabel htmlFor="gh-connect-org-compact">
                GitHub Organization <span className="opacity-70">(optional)</span>
              </FieldLabel>
              <Input
                id="gh-connect-org-compact"
                size="compact"
                type="text"
                value={organization}
                onChange={(e) => setOrganization(e.target.value)}
                placeholder="Personal account if empty"
                autoComplete="off"
              />
            </Field>
            <Button
              type="button"
              variant="primary"
              size="compact"
              leadingIcon={ExternalLink}
              onClick={(e) => void handleConnect(e)}
            >
              Connect GitHub
            </Button>
            {onUseGitUrl && (
              <Button type="button" variant="tertiary" size="compact" onClick={onUseGitUrl}>
                Use Git URL instead
              </Button>
            )}
          </div>
        )}
      </div>
    )
  }

  return (
    <div className="space-y-4">
      {error && (
        <Banner role="alert" status="error" contrast="high">
          <BannerTitle>Could not connect GitHub</BannerTitle>
          <BannerDescription>{error}</BannerDescription>
        </Banner>
      )}

      <div className="grid gap-6 md:grid-cols-[1fr_minmax(0,22rem)] md:gap-8">
        <ol className="grid gap-4 text-[13px]">
          {[
            [
              'Create the GitHub App',
              'Opens GitHub in your browser to register a private App with read-only code and push webhook permissions.',
            ],
            [
              'Install on repositories',
              'Choose which repositories or organizations the App can access on GitHub.',
            ],
            [
              'Deploy with branch picker & push triggers',
              'Deploy directly from Rig Desktop with searchable repositories, branches, and auto-deploy on push.',
            ],
          ].map(([title, body], i) => (
            <li key={title} className="flex gap-3">
              <span className="grid size-6 shrink-0 place-items-center rounded-full bg-hover font-mono text-[12px] font-medium text-muted-foreground tabular-nums">
                {i + 1}
              </span>
              <span>
                <span className="block font-medium text-foreground">{title}</span>
                <span className="mt-0.5 block text-muted-foreground">{body}</span>
              </span>
            </li>
          ))}
        </ol>

        <form
          onSubmit={(e) => void handleConnect(e)}
          className="space-y-4 md:border-l md:border-border/80 md:pl-7"
        >
          <Field>
            <FieldLabel htmlFor="gh-connect-org">
              GitHub Organization <span className="opacity-70">(optional)</span>
            </FieldLabel>
            <Input
              id="gh-connect-org"
              type="text"
              value={organization}
              onChange={(e) => setOrganization(e.target.value)}
              placeholder="Personal account if empty"
              autoComplete="off"
              disabled={connecting}
            />
            <FieldDescription>GitHub org that will own the App.</FieldDescription>
          </Field>

          {connecting ? (
            <div className="space-y-2.5 rounded-lg border border-primary/30 bg-primary/8 p-3.5">
              <div className="flex items-center gap-2 text-[13px] font-medium text-foreground">
                <Loader2 className="size-4 shrink-0 animate-spin text-primary" />
                Waiting for GitHub callback…
              </div>
              <p className="text-[12px] text-muted-foreground">
                Complete the form in your browser. Rig Desktop will capture the callback
                automatically.
              </p>
              <Button
                type="button"
                variant="tertiary"
                size="compact"
                leadingIcon={X}
                onClick={() => void handleCancel()}
              >
                Cancel
              </Button>
            </div>
          ) : (
            <Button type="submit" variant="primary" leadingIcon={ExternalLink} className="w-full">
              Create & Connect GitHub App
            </Button>
          )}
        </form>
      </div>
    </div>
  )
}
