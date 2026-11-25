import { useMutation, useQueryClient } from '@tanstack/react-query'
import { createFileRoute, Link, useNavigate } from '@tanstack/react-router'
import { useEffect, useRef } from 'react'
import { toast } from 'sonner'
import { api, unwrap } from '@/api/client'
import { Page, Panel } from '@rig/ui/app/page'
import { ErrorState } from '@/components/app/state'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'

const str = (v: unknown) => (typeof v === 'string' && v.length <= 200 ? v : undefined)

/** GitHub redirects here after the App is created, with a one-time code. */
export const Route = createFileRoute('/_app/settings/github/callback')({
  staticData: { crumb: 'GitHub' },
  validateSearch: (search: Record<string, unknown>): { code?: string; state?: string } => ({
    code: str(search.code),
    state: str(search.state),
  }),
  component: Callback,
})

function Callback() {
  const { code, state } = Route.useSearch()
  const navigate = useNavigate()
  const qc = useQueryClient()
  const started = useRef(false)

  const complete = useMutation({
    mutationFn: (body: { code: string; state: string }) =>
      unwrap(api.POST('/api/v1/github/manifest/complete', { body })),
    meta: { quietError: true },
    onSuccess: (status) => {
      qc.setQueryData(['github'], status)
      toast.success('GitHub connected', { description: 'Now install the App on your repositories.' })
      void navigate({ to: '/settings/github', replace: true })
    },
  })

  useEffect(() => {
    // Strict mode runs effects twice; the code is single-use.
    if (started.current || !code || !state) return
    started.current = true
    complete.mutate({ code, state })
  }, [code, state, complete])

  return (
    <Page title="Connecting GitHub">
      <Panel>
        {!code || !state ? (
          <ErrorState error={new Error('GitHub did not send back a code. Start again from Settings → GitHub.')} />
        ) : complete.isError ? (
          <ErrorState error={complete.error} />
        ) : (
          <div className="flex items-center gap-3 text-sm text-muted-foreground">
            <Spinner />
            Saving the App's keys…
          </div>
        )}
        {(complete.isError || !code || !state) && (
          <Button variant="outline" className="mt-4" nativeButton={false} render={<Link to="/settings/github" />}>
            Back to GitHub settings
          </Button>
        )}
      </Panel>
    </Page>
  )
}
