import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { ExternalLink, Globe, Plus, Trash2 } from 'lucide-react'
import { useState, type FormEvent } from 'react'

import { api, unwrap, type Schemas } from '@/api/client'
import { domainsQuery, meQuery, networkQuery } from '@/api/queries'
import { Panel } from '@/components/app/page'
import { EmptyState, ErrorState } from '@/components/app/state'
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
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Field, FieldDescription, FieldGroup, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
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

const ACCESS_LABEL: Record<Access, string> = {
  public: 'Anyone',
  lan: 'Local network',
  tailscale: 'Tailnet',
}

const items = <T extends string>(labels: Record<T, string>) =>
  (Object.keys(labels) as T[]).map((value) => ({ value, label: labels[value] }))
const TLS_ITEMS = items(TLS_LABEL)
const ACCESS_ITEMS = items(ACCESS_LABEL)

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
      <div className="flex flex-wrap items-center justify-between gap-x-6 gap-y-3">
        <p className="text-muted-foreground">Addresses that route to this app. Each one gets its own certificate.</p>
        {domains.data?.length ? addButton : null}
      </div>

      {domains.isPending ? (
        <Panel>
          <div className="space-y-2">
            {Array.from({ length: 2 }, (_, i) => (
              <Skeleton key={i} className="h-8" />
            ))}
          </div>
        </Panel>
      ) : domains.isError ? (
        <ErrorState error={domains.error} onRetry={() => void domains.refetch()} />
      ) : domains.data.length === 0 ? (
        <EmptyState icon={Globe} title="No domains yet" action={addButton}>
          Add a domain to reach this app over HTTPS without remembering a port.
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
              {domains.data.map((d) => (
                <TableRow key={d.id}>
                  <TableCell className="pl-4">
                    <a
                      href={domainUrl(d)}
                      target="_blank"
                      rel="noreferrer"
                      className="inline-flex max-w-72 items-center gap-1 font-medium underline-offset-4 hover:underline"
                    >
                      <span className="truncate">
                        {d.host}
                        {d.port ? <span className="text-muted-foreground">:{d.port}</span> : null}
                      </span>
                      <ExternalLink className="size-3 shrink-0 text-muted-foreground" aria-hidden />
                    </a>
                  </TableCell>
                  <TableCell className="text-muted-foreground">{TLS_LABEL[d.tls_mode]}</TableCell>
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
      <DialogContent>
        {/* Mounted only while open, so the form always starts empty. */}
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

  // A tailnet address is one machine name, so each app needs its own port.
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
        <DialogDescription>Point the domain at this server, then add it here to start serving the app.</DialogDescription>
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
              to reach the app from your tailnet with a trusted certificate.
            </FieldDescription>
          ) : null}
        </Field>
        <div className="grid gap-4 sm:grid-cols-2">
          <Field>
            <FieldLabel htmlFor="domain-access">Reachable by</FieldLabel>
            <Select items={ACCESS_ITEMS} value={access} onValueChange={(v) => setAccess(v as Access)}>
              <SelectTrigger id="domain-access" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {ACCESS_ITEMS.map((item) => (
                  <SelectItem key={item.value} value={item.value}>
                    {item.label}
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
        <FieldDescription className="-mt-2">
          Others are turned away with 403. Leave the port empty to use the standard one
          {tls === 'tailscale' ? '; a tailnet name needs a port of its own per app' : ''}.
        </FieldDescription>
        <Field>
          <FieldLabel htmlFor="domain-tls">TLS</FieldLabel>
          <Select items={TLS_ITEMS} value={tls} onValueChange={(v) => setTls(v as TlsMode)}>
            <SelectTrigger id="domain-tls" className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {TLS_ITEMS.map((item) => (
                <SelectItem key={item.value} value={item.value}>
                  {item.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <FieldDescription>
            <span className="block">
              <strong className="font-medium text-foreground">Automatic</strong> is a public certificate from Let’s
              Encrypt. The domain’s DNS must point at this server.
            </span>
            <span className="mt-1 block">
              <strong className="font-medium text-foreground">Private CA</strong> is a certificate from Rig’s own
              authority, for private networks.
            </span>
            <span className="mt-1 block">
              <strong className="font-medium text-foreground">Tailscale</strong> is a trusted certificate for this
              machine’s .ts.net name.
            </span>
            <span className="mt-1 block">
              <strong className="font-medium text-foreground">Off</strong> serves plain HTTP.
            </span>
          </FieldDescription>
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
            The app stops answering on this address and its certificate is discarded. You can add the domain again later.
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
