import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute, redirect, useNavigate } from '@tanstack/react-router'
import { Eye, EyeOff } from 'lucide-react'
import { useState, type FormEvent } from 'react'
import { api, setCsrfToken, unwrap } from '@/api/client'
import { setupStatusQuery } from '@/api/queries'
import { AuthLayout } from '@/components/app/auth-layout'
import { Field, FieldDescription, FieldGroup, FieldLabel } from '@rig/ui/field'
import { Input } from '@rig/ui/input'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import { useSyncBranding } from '@/lib/branding'

export const Route = createFileRoute('/setup')({
  beforeLoad: async ({ context }) => {
    const status = await context.queryClient.ensureQueryData(setupStatusQuery)
    if (status.setup_complete) throw redirect({ to: '/login' })
  },
  component: SetupPage,
})

function SetupPage() {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const { data: status } = useQuery(setupStatusQuery)
  const [error, setError] = useState('')
  const [showPassword, setShowPassword] = useState(false)

  useSyncBranding(null)

  const setup = useMutation({
    mutationFn: (body: { token: string; name: string; email: string; password: string; org_name: string }) =>
      unwrap(api.POST('/api/v1/setup', { body })),
    meta: { quietError: true, success: 'Workspace is ready' },
    onSuccess: async (me) => {
      setCsrfToken(me.csrf_token)
      queryClient.setQueryData(['me'], me)
      queryClient.setQueryData(setupStatusQuery.queryKey, (old) =>
        old
          ? {
              ...old,
              setup_complete: true,
              branding: {
                org_name: me.org.name,
                logo_url: me.org.logo_url ?? null,
                accent_color: me.org.accent_color ?? 'emerald',
                env_label: me.org.env_label ?? null,
                env_color: me.org.env_color ?? null,
              },
            }
          : old,
      )
      await navigate({ to: '/' })
    },
    onError: (e) => setError(e.message),
  })

  function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault()
    setError('')
    const f = new FormData(e.currentTarget)
    const get = (k: string) => String(f.get(k) ?? '')
    setup.mutate({
      token: get('token'),
      name: get('name'),
      email: get('email'),
      password: get('password'),
      org_name: get('org_name'),
    })
  }

  return (
    <AuthLayout
      mode="setup"
      eyebrow="Initial setup"
      title="Create owner account"
      description="Enter the one-time setup token from your server console and name your organization."
      version={status?.version ?? undefined}
    >
      <form onSubmit={submit}>
        <FieldGroup>
          <Field>
            <FieldLabel htmlFor="token">Setup token</FieldLabel>
            <Input
              id="token"
              name="token"
              required
              autoFocus
              autoComplete="off"
              spellCheck={false}
              placeholder="Paste token from server console"
              className="font-mono"
            />
            <FieldDescription>Valid for 30 minutes. Run `rig-server setup-token` for a fresh one.</FieldDescription>
          </Field>
          <Field>
            <FieldLabel htmlFor="org_name">Organization name</FieldLabel>
            <Input
              id="org_name"
              name="org_name"
              required
              maxLength={80}
              placeholder="Acme Cloud"
              autoComplete="organization"
            />
          </Field>
          <div className="grid gap-4 sm:grid-cols-2">
            <Field>
              <FieldLabel htmlFor="name">Your name</FieldLabel>
              <Input id="name" name="name" required maxLength={80} placeholder="Ada Lovelace" autoComplete="name" />
            </Field>
            <Field>
              <FieldLabel htmlFor="email">Email</FieldLabel>
              <Input id="email" name="email" type="email" required placeholder="ada@acme.dev" autoComplete="username" />
            </Field>
          </div>
          <Field>
            <FieldLabel htmlFor="password">Password</FieldLabel>
            <div className="relative">
              <Input
                id="password"
                name="password"
                type={showPassword ? 'text' : 'password'}
                required
                minLength={10}
                placeholder="At least 10 characters"
                autoComplete="new-password"
                className="pr-10"
              />
              <button
                type="button"
                aria-label={showPassword ? 'Hide password' : 'Show password'}
                onClick={() => setShowPassword((v) => !v)}
                className="absolute top-1/2 right-2.5 grid size-7 -translate-y-1/2 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-hover hover:text-foreground"
              >
                {showPassword ? <EyeOff className="size-4" /> : <Eye className="size-4" />}
              </button>
            </div>
            <FieldDescription>At least 10 characters.</FieldDescription>
          </Field>
          {error ? (
            <div role="alert" className="rounded-lg border border-fail/30 bg-destructive-light px-3 py-2 text-[13px] text-fail">
              {error}
            </div>
          ) : null}
          <Button type="submit" size="lg" className="mt-1 w-full" disabled={setup.isPending}>
            {setup.isPending ? <Spinner data-icon="inline-start" /> : null}
            Create owner account
          </Button>
        </FieldGroup>
      </form>
    </AuthLayout>
  )
}
