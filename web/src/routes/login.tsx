import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute, redirect, useNavigate } from '@tanstack/react-router'
import { ArrowLeft, Eye, EyeOff, KeyRound, ShieldCheck } from 'lucide-react'
import { useState, type FormEvent } from 'react'
import { api, ApiError, setCsrfToken, unwrap, type Schemas } from '@/api/client'
import { setupStatusQuery } from '@/api/queries'
import { AuthLayout } from '@/components/app/auth-layout'
import { Field, FieldError, FieldGroup, FieldLabel } from '@/components/fluid/field'
import { Input } from '@/components/fluid/input'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import { useSyncBranding } from '@/lib/branding'
import { getDefaultHome } from '@/lib/preferences'

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
  const { data: status } = useQuery(setupStatusQuery)
  const [error, setError] = useState('')
  const [showPassword, setShowPassword] = useState(false)
  const [challenge, setChallenge] = useState<{ email: string; token: string } | null>(null)
  const [useRecovery, setUseRecovery] = useState(false)

  const branding = status?.branding
  useSyncBranding({
    orgName: branding?.org_name,
    logoUrl: branding?.logo_url,
    accentColor: branding?.accent_color,
  })

  const login = useMutation({
    mutationFn: (body: Schemas['LoginRequest']) => unwrap(api.POST('/api/v1/auth/login', { body })),
    meta: { quietError: true },
    onSuccess: async (me) => {
      setCsrfToken(me.csrf_token)
      queryClient.setQueryData(['me'], me)
      try {
        sessionStorage.setItem('rig-home-applied', '1')
      } catch {
        // Ignore storage errors.
      }
      await navigate({ to: next ?? getDefaultHome() })
    },
    onError: (e, body) => {
      if (e instanceof ApiError && e.code === 'second_factor_required' && e.challengeToken) {
        setChallenge({ email: body.email ?? '', token: e.challengeToken })
        return
      }
      if (e instanceof ApiError && e.status === 401) {
        setError(
          body.code
            ? 'That code did not work, or the sign-in session expired. Try again.'
            : 'That email and password do not match.',
        )
        return
      }
      setError(e.message)
    },
  })

  function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault()
    setError('')
    const form = new FormData(e.currentTarget)
    if (challenge) {
      login.mutate({
        email: '',
        password: '',
        challenge_token: challenge.token,
        code: String(form.get('code') ?? '').trim(),
      })
      return
    }
    login.mutate({ email: String(form.get('email')), password: String(form.get('password')) })
  }

  if (challenge) {
    return (
      <AuthLayout
        mode="login"
        eyebrow="Step 2 of 2 · Security check"
        title="Two-factor verification"
        description={
          useRecovery
            ? 'Enter one of your single-use recovery codes to complete sign-in.'
            : 'Enter the six-digit code from your authenticator app.'
        }
        branding={branding}
        version={status?.version ?? undefined}
      >
        <form onSubmit={submit} key={useRecovery ? 'recovery' : 'totp'} className="space-y-5">
          <div className="flex items-center gap-2.5 rounded-xl border border-border/80 bg-surface/60 px-3.5 py-2.5 text-[12.5px] text-muted-foreground">
            {useRecovery ? (
              <KeyRound className="size-4 shrink-0 text-primary" />
            ) : (
              <ShieldCheck className="size-4 shrink-0 text-primary" />
            )}
            <span className="truncate">
              Signing in as <span className="font-medium text-foreground">{challenge.email}</span>
            </span>
          </div>

          <FieldGroup>
            <Field data-invalid={error ? true : undefined}>
              <FieldLabel htmlFor="code">{useRecovery ? 'Recovery code' : 'Authenticator code'}</FieldLabel>
              <Input
                id="code"
                name="code"
                required
                autoFocus
                autoComplete="one-time-code"
                spellCheck={false}
                aria-invalid={error ? true : undefined}
                className="font-mono text-base tracking-[0.2em]"
                {...(useRecovery
                  ? { placeholder: 'xxxx-xxxx-xxxx-xxxx', maxLength: 32 }
                  : { inputMode: 'numeric' as const, pattern: '[0-9 ]{6,7}', placeholder: '123 456', maxLength: 7 })}
              />
              {error ? <FieldError>{error}</FieldError> : null}
            </Field>
            <Button type="submit" size="lg" className="w-full" disabled={login.isPending}>
              {login.isPending ? <Spinner data-icon="inline-start" /> : null}
              Verify and sign in
            </Button>
          </FieldGroup>

          <div className="flex items-center justify-between border-t border-border/70 pt-4 text-[13px]">
            <button
              type="button"
              className="text-muted-foreground underline-offset-4 transition-colors hover:text-foreground hover:underline"
              onClick={() => {
                setError('')
                setUseRecovery((r) => !r)
              }}
            >
              {useRecovery ? 'Use authenticator app' : 'Use a recovery code'}
            </button>
            <button
              type="button"
              className="inline-flex items-center gap-1 text-muted-foreground transition-colors hover:text-foreground"
              onClick={() => {
                setError('')
                setUseRecovery(false)
                setChallenge(null)
              }}
            >
              <ArrowLeft className="size-3.5" />
              Back
            </button>
          </div>
        </form>
      </AuthLayout>
    )
  }

  return (
    <AuthLayout
      mode="login"
      eyebrow="Sign in"
      title={branding?.org_name ? `Sign in to ${branding.org_name}` : 'Sign in'}
      description="Enter your account email and password to open the dashboard."
      branding={branding}
      version={status?.version ?? undefined}
    >
      <form onSubmit={submit} noValidate={false}>
        <FieldGroup>
          <Field>
            <FieldLabel htmlFor="email">Email</FieldLabel>
            <Input
              id="email"
              name="email"
              type="email"
              placeholder="you@company.com"
              autoComplete="username"
              required
              autoFocus
            />
          </Field>
          <Field>
            <FieldLabel htmlFor="password">Password</FieldLabel>
            <div className="relative">
              <Input
                id="password"
                name="password"
                type={showPassword ? 'text' : 'password'}
                placeholder="••••••••••"
                autoComplete="current-password"
                required
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
          </Field>
          {error ? (
            <div role="alert" className="rounded-lg border border-fail/30 bg-destructive-light px-3 py-2 text-[13px] text-fail">
              {error}
            </div>
          ) : null}
          <Button type="submit" size="lg" className="mt-1 w-full" disabled={login.isPending}>
            {login.isPending ? <Spinner data-icon="inline-start" /> : null}
            Sign in
          </Button>
        </FieldGroup>
      </form>
    </AuthLayout>
  )
}
