import { useMutation, useQueryClient } from '@tanstack/react-query'
import { createFileRoute, redirect, useNavigate } from '@tanstack/react-router'
import { useState, type FormEvent } from 'react'
import { api, ApiError, setCsrfToken, unwrap, type Schemas } from '@/api/client'
import { setupStatusQuery } from '@/api/queries'
import { AuthLayout } from '@/components/app/auth-layout'
import { Button } from '@/components/ui/button'
import { Field, FieldError, FieldGroup, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Spinner } from '@/components/ui/spinner'

/** Only same-site paths are followed after sign-in. */
function safeNext(next: unknown): string | undefined {
  return typeof next === 'string' && next.startsWith('/') && !next.startsWith('//') ? next : undefined
}

export const Route = createFileRoute('/login')({
  validateSearch: (search): { next?: string } => ({ next: safeNext(search.next) }),
  beforeLoad: async ({ context }) => {
    const status = await context.queryClient.ensureQueryData(setupStatusQuery)
    if (!status.setup_complete) throw redirect({ to: '/setup' })
  },
  component: LoginPage,
})

function LoginPage() {
  const { next } = Route.useSearch()
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const [error, setError] = useState('')
  // Kept between the two steps so the second one sends both again; the
  // server never issues a half-signed-in session.
  const [credentials, setCredentials] = useState<{ email: string; password: string } | null>(null)
  const [useRecovery, setUseRecovery] = useState(false)

  const login = useMutation({
    mutationFn: (body: Schemas['LoginRequest']) => unwrap(api.POST('/api/v1/auth/login', { body })),
    meta: { quietError: true },
    onSuccess: async (me) => {
      setCsrfToken(me.csrf_token)
      queryClient.setQueryData(['me'], me)
      await navigate({ to: next ?? '/' })
    },
    onError: (e, body) => {
      if (e instanceof ApiError && e.code === 'second_factor_required') {
        setCredentials({ email: body.email, password: body.password })
        return
      }
      if (e instanceof ApiError && e.status === 401) {
        setError(body.code ? 'That code did not work. Codes change every 30 seconds.' : 'That email and password do not match.')
        return
      }
      setError(e.message)
    },
  })

  function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault()
    setError('')
    const form = new FormData(e.currentTarget)
    if (credentials) {
      login.mutate({ ...credentials, code: String(form.get('code') ?? '').trim() })
      return
    }
    login.mutate({ email: String(form.get('email')), password: String(form.get('password')) })
  }

  if (credentials) {
    return (
      <AuthLayout
        title="Two-factor sign-in"
        description={
          useRecovery ? 'Enter one of the recovery codes you saved.' : 'Enter the six-digit code from your authenticator app.'
        }
      >
        <form onSubmit={submit} key={useRecovery ? 'recovery' : 'totp'}>
          <FieldGroup>
            <Field data-invalid={error ? true : undefined}>
              <FieldLabel htmlFor="code">{useRecovery ? 'Recovery code' : 'Code'}</FieldLabel>
              <Input
                id="code"
                name="code"
                required
                autoFocus
                autoComplete="one-time-code"
                spellCheck={false}
                aria-invalid={error ? true : undefined}
                className="font-mono tracking-widest"
                {...(useRecovery
                  ? { placeholder: 'xxxx-xxxx', maxLength: 12 }
                  : { inputMode: 'numeric' as const, pattern: '[0-9 ]{6,7}', placeholder: '123 456', maxLength: 7 })}
              />
              {error ? <FieldError>{error}</FieldError> : null}
            </Field>
            <Button type="submit" size="lg" disabled={login.isPending}>
              {login.isPending ? <Spinner data-icon="inline-start" /> : null}
              Verify
            </Button>
            <div className="flex justify-between text-sm">
              <button
                type="button"
                className="text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
                onClick={() => {
                  setError('')
                  setUseRecovery((r) => !r)
                }}
              >
                {useRecovery ? 'Use the authenticator app' : 'Use a recovery code'}
              </button>
              <button
                type="button"
                className="text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
                onClick={() => {
                  setError('')
                  setUseRecovery(false)
                  setCredentials(null)
                }}
              >
                Back
              </button>
            </div>
          </FieldGroup>
        </form>
      </AuthLayout>
    )
  }

  return (
    <AuthLayout title="Sign in" description="Use the account your administrator created for you.">
      <form onSubmit={submit} noValidate={false}>
        <FieldGroup>
          <Field>
            <FieldLabel htmlFor="email">Email</FieldLabel>
            <Input id="email" name="email" type="email" autoComplete="username" required autoFocus />
          </Field>
          <Field>
            <FieldLabel htmlFor="password">Password</FieldLabel>
            <Input id="password" name="password" type="password" autoComplete="current-password" required />
          </Field>
          {error ? <FieldError>{error}</FieldError> : null}
          <Button type="submit" size="lg" disabled={login.isPending}>
            {login.isPending ? <Spinner data-icon="inline-start" /> : null}
            Sign in
          </Button>
        </FieldGroup>
      </form>
    </AuthLayout>
  )
}
