import { useMutation, useQueryClient } from '@tanstack/react-query'
import { createFileRoute, redirect, useNavigate } from '@tanstack/react-router'
import { useState, type FormEvent } from 'react'
import { api, setCsrfToken, unwrap } from '@/api/client'
import { setupStatusQuery } from '@/api/queries'
import { AuthLayout } from '@/components/app/auth-layout'
import { Button } from '@/components/ui/button'
import { Field, FieldDescription, FieldError, FieldGroup, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Spinner } from '@/components/ui/spinner'

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
  const [error, setError] = useState('')

  const setup = useMutation({
    mutationFn: (body: { token: string; name: string; email: string; password: string; org_name: string }) =>
      unwrap(api.POST('/api/v1/setup', { body })),
    meta: { quietError: true, success: 'Rig is ready' },
    onSuccess: async (me) => {
      setCsrfToken(me.csrf_token)
      queryClient.setQueryData(['me'], me)
      queryClient.setQueryData(setupStatusQuery.queryKey, (old) => (old ? { ...old, setup_complete: true } : old))
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
      title="Set up Rig"
      description="Create the owner account. The setup token was printed in the server's console when it started."
    >
      <form onSubmit={submit}>
        <FieldGroup>
          <Field>
            <FieldLabel htmlFor="token">Setup token</FieldLabel>
            <Input id="token" name="token" required autoFocus autoComplete="off" spellCheck={false} className="font-mono" />
            <FieldDescription>Valid for 30 minutes. Restart the server to get a new one.</FieldDescription>
          </Field>
          <Field>
            <FieldLabel htmlFor="org_name">Organization name</FieldLabel>
            <Input id="org_name" name="org_name" required maxLength={80} autoComplete="organization" />
          </Field>
          <Field>
            <FieldLabel htmlFor="name">Your name</FieldLabel>
            <Input id="name" name="name" required maxLength={80} autoComplete="name" />
          </Field>
          <Field>
            <FieldLabel htmlFor="email">Email</FieldLabel>
            <Input id="email" name="email" type="email" required autoComplete="username" />
          </Field>
          <Field>
            <FieldLabel htmlFor="password">Password</FieldLabel>
            <Input id="password" name="password" type="password" required minLength={10} autoComplete="new-password" />
            <FieldDescription>At least 10 characters.</FieldDescription>
          </Field>
          {error ? <FieldError>{error}</FieldError> : null}
          <Button type="submit" size="lg" disabled={setup.isPending}>
            {setup.isPending ? <Spinner data-icon="inline-start" /> : null}
            Create owner account
          </Button>
        </FieldGroup>
      </form>
    </AuthLayout>
  )
}
