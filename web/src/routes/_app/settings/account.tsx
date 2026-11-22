import { queryOptions, useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { useState, type FormEvent } from 'react'
import { api, unwrap, type Schemas } from '@/api/client'
import { meQuery } from '@/api/queries'
import { CopyButton } from '@/components/app/copy-button'
import { Page, Panel } from '@/components/app/page'
import { Badge } from '@/components/ui/badge'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Skeleton } from '@/components/ui/skeleton'
import { Button } from '@/components/ui/button'
import { Field, FieldDescription, FieldError, FieldGroup, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Spinner } from '@/components/ui/spinner'

export const Route = createFileRoute('/_app/settings/account')({
  staticData: { crumb: 'Account' },
  component: AccountPage,
})

function AccountPage() {
  const me = useQuery(meQuery).data
  return (
    <Page title="Account" description="Your sign-in details for this Rig." className="max-w-2xl">
      <Panel title="Profile">
        <dl className="grid grid-cols-[6rem_1fr] gap-y-2">
          <dt className="text-muted-foreground">Name</dt>
          <dd className="truncate">{me?.user.name}</dd>
          <dt className="text-muted-foreground">Email</dt>
          <dd className="truncate">{me?.user.email}</dd>
          <dt className="text-muted-foreground">Role</dt>
          <dd>
            <span className="capitalize">{me?.role}</span> in {me?.org.name}
          </dd>
        </dl>
      </Panel>
      <PasswordPanel />
      <TwoFactorPanel />
    </Page>
  )
}

function PasswordPanel() {
  const [mismatch, setMismatch] = useState(false)
  // Remounting the form is the simplest way to clear three password fields.
  const [formKey, setFormKey] = useState(0)
  const change = useMutation({
    mutationFn: (body: Schemas['ChangePasswordRequest']) => unwrap(api.POST('/api/v1/auth/password', { body })),
    meta: { success: 'Password changed. Other sessions were signed out.' },
    onSuccess: () => setFormKey((k) => k + 1),
  })

  function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault()
    const form = new FormData(e.currentTarget)
    const next = String(form.get('new') ?? '')
    if (next !== String(form.get('confirm') ?? '')) {
      setMismatch(true)
      return
    }
    setMismatch(false)
    change.mutate({ current_password: String(form.get('current') ?? ''), new_password: next })
  }

  return (
    <Panel title="Password" description="Changing it signs out your other browsers. Access tokens keep working.">
      <form key={formKey} onSubmit={submit}>
        <FieldGroup>
          <Field>
            <FieldLabel htmlFor="pw-current">Current password</FieldLabel>
            <Input id="pw-current" name="current" type="password" required autoComplete="current-password" />
          </Field>
          <Field>
            <FieldLabel htmlFor="pw-new">New password</FieldLabel>
            <Input id="pw-new" name="new" type="password" required minLength={10} autoComplete="new-password" />
            <FieldDescription>At least 10 characters.</FieldDescription>
          </Field>
          <Field data-invalid={mismatch || undefined}>
            <FieldLabel htmlFor="pw-confirm">Repeat new password</FieldLabel>
            <Input
              id="pw-confirm"
              name="confirm"
              type="password"
              required
              autoComplete="new-password"
              aria-invalid={mismatch || undefined}
              onChange={() => mismatch && setMismatch(false)}
            />
            {mismatch ? <FieldError>The two new passwords do not match.</FieldError> : null}
          </Field>
          <div>
            <Button type="submit" disabled={change.isPending}>
              {change.isPending ? <Spinner data-icon="inline-start" /> : null}
              Change password
            </Button>
          </div>
        </FieldGroup>
      </form>
    </Panel>
  )
}

const totpQuery = queryOptions({
  queryKey: ['account', 'totp'],
  queryFn: () => unwrap(api.GET('/api/v1/account/totp')),
})

type Step =
  | { kind: 'idle' }
  | { kind: 'password' }
  | { kind: 'scan'; enrollment: Schemas['TotpEnrollment'] }
  | { kind: 'codes'; codes: string[] }
  | { kind: 'confirm'; action: 'disable' | 'regenerate' }

function TwoFactorPanel() {
  const queryClient = useQueryClient()
  const status = useQuery(totpQuery)
  const [step, setStep] = useState<Step>({ kind: 'idle' })
  const close = () => setStep({ kind: 'idle' })
  const refresh = () => queryClient.invalidateQueries({ queryKey: totpQuery.queryKey })

  const setup = useMutation({
    mutationFn: (password: string) => unwrap(api.POST('/api/v1/account/totp/setup', { body: { password } })),
    onSuccess: (enrollment) => setStep({ kind: 'scan', enrollment }),
  })
  const enable = useMutation({
    mutationFn: (code: string) => unwrap(api.POST('/api/v1/account/totp/enable', { body: { code } })),
    meta: { success: 'Two-factor sign-in is on. Other sessions were signed out.' },
    onSuccess: ({ codes }) => {
      setStep({ kind: 'codes', codes })
      void refresh()
    },
  })
  const disable = useMutation({
    mutationFn: (body: Schemas['TotpDisable']) => unwrap(api.POST('/api/v1/account/totp/disable', { body })),
    meta: { success: 'Two-factor sign-in is off.' },
    onSuccess: () => {
      close()
      void refresh()
    },
  })
  const regenerate = useMutation({
    mutationFn: (body: Schemas['TotpDisable']) => unwrap(api.POST('/api/v1/account/totp/recovery-codes', { body })),
    meta: { success: 'New recovery codes made. The old ones no longer work.' },
    onSuccess: ({ codes }) => {
      setStep({ kind: 'codes', codes })
      void refresh()
    },
  })

  const data = status.data
  return (
    <Panel
      title="Two-factor sign-in"
      description="Ask for a code from an authenticator app after your password."
      actions={
        data ? (
          data.enabled ? (
            <Badge variant="secondary">On</Badge>
          ) : (
            <Badge variant="outline">Off</Badge>
          )
        ) : null
      }
    >
      {!data ? (
        <Skeleton className="h-9 w-40" />
      ) : data.enabled ? (
        <div className="flex flex-col gap-3">
          <p className="text-sm text-muted-foreground">
            {data.recovery_codes_left} of 10 recovery codes left.
            {data.recovery_codes_left <= 3 ? ' Make new ones before you run out.' : ''}
          </p>
          <div className="flex flex-wrap gap-2">
            <Button variant="outline" onClick={() => setStep({ kind: 'confirm', action: 'regenerate' })}>
              New recovery codes
            </Button>
            <Button variant="outline" onClick={() => setStep({ kind: 'confirm', action: 'disable' })}>
              Turn off
            </Button>
          </div>
        </div>
      ) : (
        <div className="flex flex-col gap-3">
          <p className="text-sm text-muted-foreground">
            Works with any TOTP app: 1Password, Bitwarden, Google Authenticator, Aegis and others. API tokens are not affected.
          </p>
          <div>
            <Button onClick={() => setStep({ kind: 'password' })}>Turn on</Button>
          </div>
        </div>
      )}

      <Dialog open={step.kind !== 'idle'} onOpenChange={(open) => !open && step.kind !== 'codes' && close()}>
        <DialogContent className="sm:max-w-md">
          {step.kind === 'password' ? (
            <PasswordStep pending={setup.isPending} onSubmit={(password) => setup.mutate(password)} />
          ) : step.kind === 'scan' ? (
            <ScanStep enrollment={step.enrollment} pending={enable.isPending} onSubmit={(code) => enable.mutate(code)} />
          ) : step.kind === 'codes' ? (
            <CodesStep codes={step.codes} onDone={close} />
          ) : step.kind === 'confirm' ? (
            <ConfirmStep
              action={step.action}
              pending={disable.isPending || regenerate.isPending}
              onSubmit={(body) => (step.action === 'disable' ? disable.mutate(body) : regenerate.mutate(body))}
            />
          ) : null}
        </DialogContent>
      </Dialog>
    </Panel>
  )
}

function PasswordStep({ pending, onSubmit }: { pending: boolean; onSubmit: (password: string) => void }) {
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault()
        onSubmit(String(new FormData(e.currentTarget).get('password') ?? ''))
      }}
    >
      <DialogHeader>
        <DialogTitle>Confirm your password</DialogTitle>
        <DialogDescription>Turning on two-factor sign-in needs your current password.</DialogDescription>
      </DialogHeader>
      <Field className="my-4">
        <FieldLabel htmlFor="tfa-password">Password</FieldLabel>
        <Input id="tfa-password" name="password" type="password" required autoFocus autoComplete="current-password" />
      </Field>
      <DialogFooter>
        <Button type="submit" disabled={pending}>
          {pending ? <Spinner data-icon="inline-start" /> : null}
          Continue
        </Button>
      </DialogFooter>
    </form>
  )
}

function ScanStep({
  enrollment,
  pending,
  onSubmit,
}: {
  enrollment: Schemas['TotpEnrollment']
  pending: boolean
  onSubmit: (code: string) => void
}) {
  // The SVG comes from our own server; an <img> keeps it inert all the same.
  const src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(enrollment.qr_svg)}`
  const grouped = enrollment.secret.match(/.{1,4}/g)?.join(' ') ?? enrollment.secret
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault()
        onSubmit(String(new FormData(e.currentTarget).get('code') ?? '').trim())
      }}
    >
      <DialogHeader>
        <DialogTitle>Scan with your authenticator</DialogTitle>
        <DialogDescription>Then enter the six-digit code it shows to finish.</DialogDescription>
      </DialogHeader>
      <div className="my-4 flex flex-col items-center gap-3">
        <img src={src} alt="QR code for your authenticator app" width={200} height={200} className="rounded-md border bg-white p-1" />
        <details className="w-full text-sm">
          <summary className="cursor-pointer text-muted-foreground">Can't scan? Enter the key by hand</summary>
          <div className="mt-2 flex items-center gap-2">
            <code className="min-w-0 flex-1 break-all rounded bg-muted px-2 py-1 font-mono text-xs">{grouped}</code>
            <CopyButton value={enrollment.secret} label="Copy key" />
          </div>
        </details>
      </div>
      <Field>
        <FieldLabel htmlFor="tfa-code">Code</FieldLabel>
        <Input
          id="tfa-code"
          name="code"
          required
          autoFocus
          inputMode="numeric"
          autoComplete="one-time-code"
          pattern="[0-9 ]{6,7}"
          maxLength={7}
          placeholder="123 456"
          className="font-mono tracking-widest"
        />
      </Field>
      <DialogFooter className="mt-4">
        <Button type="submit" disabled={pending}>
          {pending ? <Spinner data-icon="inline-start" /> : null}
          Turn on
        </Button>
      </DialogFooter>
    </form>
  )
}

function CodesStep({ codes, onDone }: { codes: string[]; onDone: () => void }) {
  const text = codes.join('\n')
  return (
    <>
      <DialogHeader>
        <DialogTitle>Save your recovery codes</DialogTitle>
        <DialogDescription>
          Each one signs you in once if you lose your authenticator. They are shown only now.
        </DialogDescription>
      </DialogHeader>
      <ul className="my-4 grid grid-cols-2 gap-x-6 gap-y-1 rounded-md bg-muted p-3 font-mono text-sm" data-testid="recovery-codes">
        {codes.map((c) => (
          <li key={c}>{c}</li>
        ))}
      </ul>
      <DialogFooter>
        <CopyButton value={text} label="Copy codes" />
        <Button
          variant="outline"
          onClick={() => {
            const url = URL.createObjectURL(new Blob([`Rig recovery codes\n\n${text}\n`], { type: 'text/plain' }))
            const a = Object.assign(document.createElement('a'), { href: url, download: 'rig-recovery-codes.txt' })
            a.click()
            URL.revokeObjectURL(url)
          }}
        >
          Download
        </Button>
        <Button onClick={onDone}>I saved them</Button>
      </DialogFooter>
    </>
  )
}

function ConfirmStep({
  action,
  pending,
  onSubmit,
}: {
  action: 'disable' | 'regenerate'
  pending: boolean
  onSubmit: (body: Schemas['TotpDisable']) => void
}) {
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault()
        const form = new FormData(e.currentTarget)
        onSubmit({ password: String(form.get('password') ?? ''), code: String(form.get('code') ?? '').trim() })
      }}
    >
      <DialogHeader>
        <DialogTitle>{action === 'disable' ? 'Turn off two-factor sign-in' : 'Make new recovery codes'}</DialogTitle>
        <DialogDescription>
          {action === 'disable'
            ? 'Your password will be enough to sign in again. Other sessions are signed out.'
            : 'Your current recovery codes stop working.'}
        </DialogDescription>
      </DialogHeader>
      <FieldGroup className="my-4">
        <Field>
          <FieldLabel htmlFor="tfa-confirm-password">Password</FieldLabel>
          <Input id="tfa-confirm-password" name="password" type="password" required autoFocus autoComplete="current-password" />
        </Field>
        <Field>
          <FieldLabel htmlFor="tfa-confirm-code">Authenticator or recovery code</FieldLabel>
          <Input id="tfa-confirm-code" name="code" required autoComplete="one-time-code" className="font-mono" />
        </Field>
      </FieldGroup>
      <DialogFooter>
        <Button type="submit" variant={action === 'disable' ? 'destructive' : 'default'} disabled={pending}>
          {pending ? <Spinner data-icon="inline-start" /> : null}
          {action === 'disable' ? 'Turn off' : 'Make new codes'}
        </Button>
      </DialogFooter>
    </form>
  )
}
