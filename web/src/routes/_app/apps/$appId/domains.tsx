import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { ExternalLink, Globe, Plus, Trash2 } from 'lucide-react'
import { useState, type FormEvent } from 'react'

import { api, unwrap, type Schemas } from '@/api/client'
import { domainsQuery, meQuery, networkQuery } from '@/api/queries'
import { Panel } from '@rig/ui/app/page'
import { EmptyState, ErrorState } from '@/components/app/state'
import { Badge } from '@rig/ui/badge'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@rig/ui/dialog'
import { Field, FieldDescription, FieldGroup, FieldLabel } from '@rig/ui/field'
import { Input } from '@rig/ui/input'
import { Select, SelectContent, SelectItem, SelectTrigger } from '@rig/ui/select'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@rig/ui/table'
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

type Domain = Schemas['Domain']
type TlsMode = Schemas['TlsMode']
type Access = Schemas['Access']

const TLS_LABEL: Record<TlsMode, string> = {
  auto: 'Automatic',
  internal: 'Private CA',
  tailscale: 'Tailscale',
  off: 'Off',
}

const TLS_HINT: Record<TlsMode, string> = {
  auto: 'Public certificate via Let’s Encrypt. Point DNS at this server first.',
  internal: 'Signed by Rig’s built-in CA for private networks.',
  tailscale: 'Trusted HTTPS certificate for this machine’s .ts.net name.',
  off: 'Plain HTTP without TLS.',
}

const ACCESS_LABEL: Record<Access, string> = {
  public: 'Anyone',
  lan: 'Local network',
  tailscale: 'Tailnet',
}

const TLS_KEYS = Object.keys(TLS_LABEL) as TlsMode[]
const ACCESS_KEYS = Object.keys(ACCESS_LABEL) as Access[]

function domainUrl(d: Pick<Domain, 'host' | 'tls_mode' | 'port'>) {
  return `${d.tls_mode === 'off' ? 'http' : 'https'}://${d.host}${d.port ? `:${d.port}` : ''}`
}

export const Route = createFileRoute('/_app/apps/$appId/domains')({
  staticData: { crumb: 'Domains' },
  component: AppDomains,
})

function AppDomains() {
  const { appId } = Route.useParams()
  const domains = useQuery(domainsQuery(appId))
  const { data: me } = useQuery(meQuery)
  const canEdit = me ? hasRole(me.role, 'developer') : false

  const [adding, setAdding] = useState(false)
  const [removing, setRemoving] = useState<Domain | null>(null)

  const addButton = canEdit ? (
    <Button onClick={() => setAdding(true)}>
      <Plus data-icon="inline-start" />
      Add domain
    </Button>
  ) : undefined

  return (
    <div className="flex flex-col gap-4">
      {domains.data?.length ? (
        <div className="flex items-center justify-end">{addButton}</div>
      ) : null}

      {domains.isPending ? (
        <Panel>
          <div className="space-y-2">
            {Array.from({ length: 2 }, (_, i) => (
              <Skeleton key={i} className="h-9" />
            ))}
          </div>
        </Panel>
      ) : domains.isError ? (
        <ErrorState error={domains.error} onRetry={() => void domains.refetch()} />
      ) : domains.data.length === 0 ? (
        <EmptyState icon={Globe} title="No domains yet" action={addButton}>
          Add a domain to route HTTPS traffic to this app with automatic certificates.
        </EmptyState>
      ) : (
        <Panel flush>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="pl-4">Domain</TableHead>
                <TableHead>TLS</TableHead>
                <TableHead className="hidden md:table-cell">Reachable by</TableHead>
                <TableHead className="hidden sm:table-cell">Added</TableHead>
                <TableHead className="w-10 pr-4" />
              </TableRow>
            </TableHeader>
            <TableBody>
              {domains.data.map((d, index) => (
                <TableRow key={d.id} index={index}>
                  <TableCell className="pl-4">
                    <a
                      href={domainUrl(d)}
                      target="_blank"
                      rel="noreferrer"
                      className="inline-flex max-w-72 items-center gap-1.5 font-mono text-[13px] font-medium text-foreground underline-offset-4 hover:underline"
                    >
                      <span className="truncate">
                        {d.host}
                        {d.port ? <span className="text-muted-foreground">:{d.port}</span> : null}
                      </span>
                      <ExternalLink className="size-3 shrink-0 text-muted-foreground" aria-hidden />
                    </a>
                  </TableCell>
                  <TableCell>
                    <Badge variant="solid" color={d.tls_mode === 'off' ? 'gray' : 'emerald'} size="compact">
                      {TLS_LABEL[d.tls_mode]}
                    </Badge>
                  </TableCell>
                  <TableCell className="hidden text-muted-foreground md:table-cell">{ACCESS_LABEL[d.access]}</TableCell>
                  <TableCell className="hidden text-muted-foreground sm:table-cell">{timeAgo(d.created_at)}</TableCell>
                  <TableCell className="pr-4 text-right">
                    {canEdit ? (
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        aria-label={`Remove ${d.host}`}
                        onClick={() => setRemoving(d)}
                      >
                        <Trash2 />
                      </Button>
                    ) : null}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </Panel>
      )}

      <AddDomainDialog appId={appId} open={adding} onOpenChange={setAdding} />
      <RemoveDomainDialog appId={appId} domain={removing} onClose={() => setRemoving(null)} />
    </div>
  )
}

function AddDomainDialog({
  appId,
  open,
  onOpenChange,
}: {
  appId: string
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent size="lg">
        {open ? <AddDomainForm appId={appId} onClose={() => onOpenChange(false)} /> : null}
      </DialogContent>
    </Dialog>
  )
}

function AddDomainForm({ appId, onClose }: { appId: string; onClose: () => void }) {
  const queryClient = useQueryClient()
  const [tls, setTls] = useState<TlsMode>('auto')
  const [access, setAccess] = useState<Access>('public')
  const [host, setHost] = useState('')
  const network = useQuery(networkQuery)
  const tailnetName = network.data?.tailscale?.dns_name ?? null

  function fillTailnetName() {
    if (!tailnetName) return
    setHost(tailnetName)
    setTls('tailscale')
    setAccess('tailscale')
  }

  const add = useMutation({
    mutationFn: (body: Schemas['AddDomainRequest']) =>
      unwrap(api.POST('/api/v1/apps/{id}/domains', { params: { path: { id: appId } }, body })),
    meta: { success: 'Domain added' },
    onSuccess: async () => {
      onClose()
      await queryClient.invalidateQueries({ queryKey: ['apps', appId] })
    },
  })

  function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault()
    const portText = String(new FormData(e.currentTarget).get('port') ?? '').trim()
    add.mutate({
      host: host.trim().toLowerCase(),
      tls_mode: tls,
      access,
      port: portText ? Number(portText) : null,
    })
  }

  return (
    <form onSubmit={submit} className="contents">
      <DialogHeader>
        <DialogTitle>Add domain</DialogTitle>
        <DialogDescription>Point DNS at this server before enabling automatic TLS.</DialogDescription>
      </DialogHeader>
      <FieldGroup>
        <Field>
          <FieldLabel htmlFor="domain-host">Host</FieldLabel>
          <Input
            id="domain-host"
            name="host"
            required
            maxLength={253}
            placeholder="app.example.com"
            value={host}
            onChange={(e) => setHost(e.target.value)}
            autoFocus
            autoCapitalize="none"
            autoComplete="off"
            spellCheck={false}
          />
          {tailnetName && host !== tailnetName ? (
            <FieldDescription>
              <button
                type="button"
                className="font-medium text-foreground underline underline-offset-4"
                onClick={fillTailnetName}
              >
                Use {tailnetName}
              </button>{' '}
              for Tailscale access.
            </FieldDescription>
          ) : null}
        </Field>
        <div className="grid gap-4 sm:grid-cols-2">
          <Field>
            <FieldLabel htmlFor="domain-access">Reachable by</FieldLabel>
            <Select value={access} onValueChange={(v) => setAccess(v as Access)}>
              <SelectTrigger id="domain-access" aria-label="Reachable by" className="w-full" />
              <SelectContent>
                {ACCESS_KEYS.map((key, i) => (
                  <SelectItem key={key} index={i} value={key}>
                    {ACCESS_LABEL[key]}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </Field>
          <Field>
            <FieldLabel htmlFor="domain-port">Port</FieldLabel>
            <Input
              id="domain-port"
              name="port"
              type="number"
              inputMode="numeric"
              min={1024}
              max={65535}
              placeholder={tls === 'off' ? '80' : '443'}
              required={tls === 'tailscale'}
            />
          </Field>
        </div>
        {tls === 'tailscale' ? (
          <FieldDescription className="-mt-1.5">
            Each app on your tailnet hostname needs its own port.
          </FieldDescription>
        ) : null}
        <Field>
          <FieldLabel htmlFor="domain-tls">TLS</FieldLabel>
          <Select value={tls} onValueChange={(v) => setTls(v as TlsMode)}>
            <SelectTrigger id="domain-tls" aria-label="TLS" className="w-full" />
            <SelectContent>
              {TLS_KEYS.map((key, i) => (
                <SelectItem key={key} index={i} value={key}>
                  {TLS_LABEL[key]}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <FieldDescription>{TLS_HINT[tls]}</FieldDescription>
        </Field>
      </FieldGroup>
      <DialogFooter>
        <Button type="button" variant="outline" onClick={onClose}>
          Cancel
        </Button>
        <Button type="submit" disabled={add.isPending}>
          {add.isPending ? <Spinner data-icon="inline-start" /> : null}
          Add domain
        </Button>
      </DialogFooter>
    </form>
  )
}

function RemoveDomainDialog({
  appId,
  domain,
  onClose,
}: {
  appId: string
  domain: Domain | null
  onClose: () => void
}) {
  const queryClient = useQueryClient()
  const remove = useMutation({
    mutationFn: (domainId: string) =>
      unwrap(
        api.DELETE('/api/v1/apps/{id}/domains/{domain_id}', { params: { path: { id: appId, domain_id: domainId } } }),
      ),
    meta: { success: 'Domain removed' },
    onSuccess: async () => {
      onClose()
      await queryClient.invalidateQueries({ queryKey: ['apps', appId] })
    },
  })

  return (
    <AlertDialog open={domain !== null} onOpenChange={(open) => (open ? undefined : onClose())}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Remove {domain?.host}?</AlertDialogTitle>
          <AlertDialogDescription>
            This address stops routing to the app and its certificate is discarded. You can add it back later.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Keep domain</AlertDialogCancel>
          <AlertDialogAction
            variant="destructive"
            disabled={remove.isPending}
            onClick={() => domain && remove.mutate(domain.id)}
          >
            Remove domain
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
