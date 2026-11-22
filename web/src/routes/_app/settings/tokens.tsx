import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { KeyRound, Plus, Trash2 } from 'lucide-react'
import { useState, type FormEvent } from 'react'
import { api, unwrap, type Schemas } from '@/api/client'
import { meQuery, tokensQuery } from '@/api/queries'
import { Page, Panel } from '@/components/app/page'
import { EmptyState, ErrorState } from '@/components/app/state'
import { Badge } from '@/components/fluid/badge'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/fluid/dialog'
import { Field, FieldDescription, FieldGroup, FieldLabel } from '@/components/fluid/field'
import { Input } from '@/components/fluid/input'
import { InputCopy } from '@/components/fluid/input-copy'
import { Select, SelectContent, SelectItem, SelectTrigger } from '@/components/fluid/select'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/fluid/table'
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
import { hasRole, ROLES } from '@/lib/nav'

type Role = Schemas['Role']
type Token = Schemas['ApiToken']

const EXPIRY = [
  { value: '30', label: '30 days' },
  { value: '90', label: '90 days' },
  { value: '365', label: '1 year' },
  { value: 'never', label: 'No expiry' },
]

export const Route = createFileRoute('/_app/settings/tokens')({
  staticData: { crumb: 'Access tokens' },
  loader: ({ context }) => void context.queryClient.prefetchQuery(tokensQuery),
  component: TokensPage,
})

function isExpired(expiresAt: string | null | undefined): boolean {
  if (!expiresAt) return false
  return new Date(expiresAt).getTime() < Date.now()
}

function TokensPage() {
  const tokens = useQuery(tokensQuery)
  const [creating, setCreating] = useState(false)
  const [created, setCreated] = useState<Schemas['CreatedApiToken'] | null>(null)
  const [revoking, setRevoking] = useState<Token | null>(null)

  const newButton = (
    <Button onClick={() => setCreating(true)}>
      <Plus data-icon="inline-start" />
      New token
    </Button>
  )

  return (
    <Page
      title="Access tokens"
      description="Personal tokens for the CLI, MCP clients, and automation."
      actions={tokens.data?.length ? newButton : undefined}
      className="max-w-4xl"
    >
      {tokens.isPending ? (
        <Panel>
          <div className="space-y-2">
            {Array.from({ length: 3 }, (_, i) => (
              <Skeleton key={i} className="h-9" />
            ))}
          </div>
        </Panel>
      ) : tokens.isError ? (
        <ErrorState error={tokens.error} onRetry={() => void tokens.refetch()} />
      ) : tokens.data.length === 0 ? (
        <EmptyState icon={KeyRound} title="No access tokens" action={newButton}>
          Create a token to authenticate the Rig CLI or an MCP client.
        </EmptyState>
      ) : (
        <Panel flush>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="pl-4">Name</TableHead>
                <TableHead>Token</TableHead>
                <TableHead>Role</TableHead>
                <TableHead className="hidden sm:table-cell">Last used</TableHead>
                <TableHead className="hidden md:table-cell">Expires</TableHead>
                <TableHead className="w-10 pr-4" />
              </TableRow>
            </TableHeader>
            <TableBody>
              {tokens.data.map((t, index) => {
                const expired = isExpired(t.expires_at)
                return (
                  <TableRow key={t.id} index={index}>
                    <TableCell className="max-w-48 truncate pl-4 font-medium text-foreground">{t.name}</TableCell>
                    <TableCell>
                      <code className="text-[12.5px] text-muted-foreground">{t.prefix}…</code>
                    </TableCell>
                    <TableCell>
                      <Badge variant="solid" color="gray" size="compact" className="capitalize">
                        {t.role}
                      </Badge>
                    </TableCell>
                    <TableCell className="hidden text-muted-foreground sm:table-cell">{timeAgo(t.last_used_at)}</TableCell>
                    <TableCell className="hidden md:table-cell">
                      {expired ? (
                        <Badge variant="solid" color="amber" size="compact">
                          Expired
                        </Badge>
                      ) : (
                        <span className="text-muted-foreground">{t.expires_at ? timeAgo(t.expires_at) : 'Never'}</span>
                      )}
                    </TableCell>
                    <TableCell className="pr-4 text-right">
                      <Button variant="ghost" size="icon-sm" aria-label={`Revoke ${t.name}`} onClick={() => setRevoking(t)}>
                        <Trash2 />
                      </Button>
                    </TableCell>
                  </TableRow>
                )
              })}
            </TableBody>
          </Table>
        </Panel>
      )}

      <CreateTokenDialog open={creating} onOpenChange={setCreating} onCreated={setCreated} />
      <SecretDialog created={created} onClose={() => setCreated(null)} />
      <RevokeDialog token={revoking} onClose={() => setRevoking(null)} />
    </Page>
  )
}

function CreateTokenDialog({
  open,
  onOpenChange,
  onCreated,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  onCreated: (t: Schemas['CreatedApiToken']) => void
}) {
  const queryClient = useQueryClient()
  const me = queryClient.getQueryData(meQuery.queryKey)
  const allowed = ROLES.filter((r) => me && hasRole(me.role, r.value))
  const [role, setRole] = useState<Role>('developer')
  const [expiry, setExpiry] = useState('90')
  const effectiveRole = allowed.some((r) => r.value === role) ? role : (allowed.at(-1)?.value ?? 'viewer')

  const create = useMutation({
    mutationFn: (body: Schemas['CreateApiTokenRequest']) => unwrap(api.POST('/api/v1/tokens', { body })),
    onSuccess: async (result) => {
      onOpenChange(false)
      onCreated(result)
      await queryClient.invalidateQueries({ queryKey: tokensQuery.queryKey })
    },
  })

  function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault()
    const name = String(new FormData(e.currentTarget).get('name') ?? '')
    create.mutate({ name, role: effectiveRole, expires_in_days: expiry === 'never' ? null : Number(expiry) })
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent size="sm">
        <form onSubmit={submit} className="contents">
          <DialogHeader>
            <DialogTitle>New access token</DialogTitle>
            <DialogDescription>The secret is displayed once right after creation.</DialogDescription>
          </DialogHeader>
          <FieldGroup>
            <Field>
              <FieldLabel htmlFor="token-name">Name</FieldLabel>
              <Input id="token-name" name="name" required maxLength={60} placeholder="Laptop CLI" autoFocus autoComplete="off" />
            </Field>
            <div className="grid gap-4 sm:grid-cols-2">
              <Field>
                <FieldLabel htmlFor="token-role">Role</FieldLabel>
                <Select value={effectiveRole} onValueChange={(v) => setRole(v as Role)}>
                  <SelectTrigger id="token-role" className="w-full" />
                  <SelectContent>
                    {allowed.map((r, i) => (
                      <SelectItem key={r.value} index={i} value={r.value}>
                        {r.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </Field>
              <Field>
                <FieldLabel htmlFor="token-expiry">Expires after</FieldLabel>
                <Select value={expiry} onValueChange={(v) => setExpiry(v)}>
                  <SelectTrigger id="token-expiry" className="w-full" />
                  <SelectContent>
                    {EXPIRY.map((x, i) => (
                      <SelectItem key={x.value} index={i} value={x.value}>
                        {x.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </Field>
            </div>
            <FieldDescription>{ROLES.find((r) => r.value === effectiveRole)?.hint}.</FieldDescription>
          </FieldGroup>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={create.isPending}>
              {create.isPending ? <Spinner data-icon="inline-start" /> : null}
              Create token
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}

function SecretDialog({ created, onClose }: { created: Schemas['CreatedApiToken'] | null; onClose: () => void }) {
  const cliCmd = `rig login --url ${window.location.origin} --token <token>`
  return (
    <Dialog open={created !== null} onOpenChange={(open) => (open ? undefined : onClose())}>
      <DialogContent size="lg">
        <DialogHeader>
          <DialogTitle>Copy your token now</DialogTitle>
          <DialogDescription>
            Shown once for “{created?.token.name}”. Rig stores only its hash.
          </DialogDescription>
        </DialogHeader>
        <div className="grid gap-3">
          <InputCopy label="Token secret" value={created?.secret ?? ''} />
          <InputCopy label="CLI sign-in command" value={cliCmd} size="compact" />
        </div>
        <DialogFooter>
          <Button onClick={onClose}>I have copied it</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function RevokeDialog({ token, onClose }: { token: Token | null; onClose: () => void }) {
  const queryClient = useQueryClient()
  const revoke = useMutation({
    mutationFn: (id: string) => unwrap(api.DELETE('/api/v1/tokens/{id}', { params: { path: { id } } })),
    meta: { success: 'Token revoked' },
    onSuccess: async () => {
      onClose()
      await queryClient.invalidateQueries({ queryKey: tokensQuery.queryKey })
    },
  })

  return (
    <AlertDialog open={token !== null} onOpenChange={(open) => (open ? undefined : onClose())}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Revoke “{token?.name}”?</AlertDialogTitle>
          <AlertDialogDescription>
            Anything using this token stops authenticating immediately. This cannot be undone.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Keep token</AlertDialogCancel>
          <AlertDialogAction
            variant="destructive"
            disabled={revoke.isPending}
            onClick={() => token && revoke.mutate(token.id)}
          >
            Revoke token
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
