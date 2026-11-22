import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { Globe, House, Network } from 'lucide-react'
import { useState, type FormEvent } from 'react'

import { api, unwrap, type Schemas } from '@/api/client'
import { meQuery, networkQuery } from '@/api/queries'
import { CopyButton } from '@/components/app/copy-button'
import { Page, Panel } from '@/components/app/page'
import { ErrorState } from '@/components/app/state'
import { Button } from '@/components/ui/button'
import { Field, FieldDescription, FieldGroup, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { cn } from 'cn'
import { hasRole } from '@/lib/nav'

type Access = Schemas['Access']
type TlsMode = Schemas['TlsMode']
type DashboardDomain = Schemas['DashboardDomain']

const TLS_ITEMS: { value: TlsMode; label: string }[] = [
  { value: 'auto', label: "Automatic (Let's Encrypt)" },
  { value: 'internal', label: 'Private CA' },
  { value: 'tailscale', label: 'Tailscale' },
  { value: 'off', label: 'Off (plain HTTP)' },
]

const MODES: { value: Access; title: string; hint: string; icon: typeof Globe }[] = [
  { value: 'public', title: 'Anywhere', hint: 'Any address that can reach this server.', icon: Globe },
  { value: 'lan', title: 'Local network', hint: 'Private addresses such as 192.168.x.x and 10.x.x.x.', icon: House },
  { value: 'tailscale', title: 'Tailnet', hint: 'Devices signed in to your Tailscale network.', icon: Network },
]

export const Route = createFileRoute('/_app/settings/network')({
  staticData: { crumb: 'Network' },
  loader: ({ context }) => void context.queryClient.prefetchQuery(networkQuery),
  component: NetworkPage,
})

function NetworkPage() {
  const network = useQuery(networkQuery)
  const { data: me } = useQuery(meQuery)

  return (
    <Page
      title="Network"
      description="Where this machine can be reached, and who may open this dashboard. Each app domain sets its own reach on the app's Domains tab."
    >
      {network.isPending ? (
        <Panel>
          <div className="space-y-2">
            <Skeleton className="h-8" />
            <Skeleton className="h-8" />
          </div>
        </Panel>
      ) : network.isError ? (
        <ErrorState error={network.error} onRetry={() => void network.refetch()} />
      ) : (
        <>
          <Panel title="Addresses">
            <dl className="grid gap-x-6 gap-y-3 text-sm sm:grid-cols-[10rem_1fr]">
              <dt className="text-muted-foreground">Local network</dt>
              <dd className="flex flex-wrap gap-2">
                {network.data.lan_addresses.length ? (
                  network.data.lan_addresses.map((ip) => <Address key={ip} value={ip} />)
                ) : (
                  <span className="text-muted-foreground">None found</span>
                )}
              </dd>
              <dt className="text-muted-foreground">Tailscale</dt>
              <dd className="flex flex-wrap gap-2">
                {network.data.tailscale ? (
                  <>
                    {network.data.tailscale.dns_name ? <Address value={network.data.tailscale.dns_name} /> : null}
                    {network.data.tailscale.addresses.map((ip) => (
                      <Address key={ip} value={ip} />
                    ))}
                  </>
                ) : (
                  <span className="text-muted-foreground">Not running on this machine</span>
                )}
              </dd>
              <dt className="text-muted-foreground">You are connecting from</dt>
              <dd className="font-mono">{network.data.your_address ?? 'unknown'}</dd>
            </dl>
          </Panel>
          <DashboardDomainPanel
            current={network.data.dashboard_domain ?? null}
            hasTailscale={network.data.tailscale !== null && network.data.tailscale !== undefined}
            canEdit={me ? hasRole(me.role, 'owner') : false}
          />
          <DashboardAccessPanel
            current={network.data.dashboard_access}
            hasTailscale={network.data.tailscale !== null && network.data.tailscale !== undefined}
            canEdit={me ? hasRole(me.role, 'owner') : false}
          />
        </>
      )}
    </Page>
  )
}

function Address({ value }: { value: string }) {
  return (
    <span className="inline-flex items-center gap-1 rounded-md border bg-muted/40 py-0.5 pr-0.5 pl-2 font-mono text-xs">
      {value}
      <CopyButton value={value} label={`Copy ${value}`} />
    </span>
  )
}

function DashboardAccessPanel({
  current,
  hasTailscale,
  canEdit,
}: {
  current: Access
  hasTailscale: boolean
  canEdit: boolean
}) {
  const queryClient = useQueryClient()
  const [choice, setChoice] = useState<Access>(current)
  const save = useMutation({
    mutationFn: (access: Access) => unwrap(api.PUT('/api/v1/system/dashboard-access', { body: { access } })),
    meta: { success: 'Dashboard access saved' },
    onSuccess: (data) => queryClient.setQueryData(networkQuery.queryKey, data),
  })

  return (
    <Panel
      title="Dashboard access"
      description={
        <>
          Requests from elsewhere get 403. This machine itself is always let in, and deploy webhooks stay open. Locked
          out? Run <code className="font-mono text-xs">rig-server dashboard-access public</code> on the server and
          restart Rig.
        </>
      }
    >
      <div role="radiogroup" aria-label="Who may open the dashboard" className="grid gap-2 sm:grid-cols-3">
        {MODES.map((m) => {
          const selected = choice === m.value
          const unavailable = m.value === 'tailscale' && !hasTailscale
          return (
            <button
              key={m.value}
              type="button"
              role="radio"
              aria-checked={selected}
              disabled={!canEdit || unavailable}
              onClick={() => setChoice(m.value)}
              className={cn(
                'flex flex-col items-start gap-1 rounded-lg border p-3 text-left transition-colors outline-none',
                'focus-visible:ring-3 focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-60',
                selected ? 'border-primary bg-primary/5' : 'hover:bg-muted/50',
              )}
            >
              <span className="flex items-center gap-2 font-medium">
                <m.icon className="size-4 text-muted-foreground" aria-hidden />
                {m.title}
                {current === m.value ? <span className="text-xs font-normal text-muted-foreground">current</span> : null}
              </span>
              <span className="text-xs text-muted-foreground">{unavailable ? 'Tailscale is not running.' : m.hint}</span>
            </button>
          )
        })}
      </div>
      {canEdit ? (
        <div className="mt-4 flex justify-end">
          <Button disabled={choice === current || save.isPending} onClick={() => save.mutate(choice)}>
            {save.isPending ? <Spinner data-icon="inline-start" /> : null}
            Save
          </Button>
        </div>
      ) : (
        <p className="mt-3 text-xs text-muted-foreground">Only the owner can change this.</p>
      )}
    </Panel>
  )
}

function DashboardDomainPanel({
  current,
  hasTailscale,
  canEdit,
}: {
  current: DashboardDomain | null
  hasTailscale: boolean
  canEdit: boolean
}) {
  const queryClient = useQueryClient()
  const [host, setHost] = useState(current?.host ?? '')
  const [tls, setTls] = useState<TlsMode>(current?.tls_mode ?? 'auto')
  const save = useMutation({
    mutationFn: (body: { host: string | null; tls_mode: TlsMode }) =>
      unwrap(api.PUT('/api/v1/system/dashboard-domain', { body })),
    meta: { success: 'Dashboard address saved' },
    onSuccess: (data) => {
      queryClient.setQueryData(networkQuery.queryKey, data)
      const next = data.dashboard_domain
      // Moving to a new name: continue there, the old address stops serving the dashboard.
      if (next && next.host !== window.location.hostname) {
        window.location.assign(`${next.tls_mode === 'off' ? 'http' : 'https'}://${next.host}/`)
      }
    },
  })
  const trimmed = host.trim().toLowerCase()
  const unchanged = trimmed === (current?.host ?? '') && tls === (current?.tls_mode ?? 'auto')

  function submit(e: FormEvent) {
    e.preventDefault()
    save.mutate({ host: trimmed || null, tls_mode: tls })
  }

  return (
    <Panel
      title="Dashboard address"
      description={
        current ? (
          <>
            The dashboard is served at{' '}
            <a className="font-mono text-xs underline underline-offset-2" href={`${current.tls_mode === 'off' ? 'http' : 'https'}://${current.host}/`}>
              {current.host}
            </a>
            . Clear the name to serve it on this machine&apos;s addresses over plain HTTP again.
          </>
        ) : (
          'The dashboard is served over plain HTTP on this machine\'s addresses. Give it a domain to get a certificate and secure cookies.'
        )
      }
    >
      <form onSubmit={submit}>
        <FieldGroup>
          <div className="grid gap-4 sm:grid-cols-[1fr_14rem]">
            <Field>
              <FieldLabel htmlFor="dashboard-host">Domain</FieldLabel>
              <Input
                id="dashboard-host"
                value={host}
                onChange={(e) => setHost(e.target.value)}
                placeholder="rig.example.com"
                autoComplete="off"
                spellCheck={false}
                disabled={!canEdit}
              />
            </Field>
            <Field>
              <FieldLabel htmlFor="dashboard-tls">HTTPS</FieldLabel>
              <Select items={TLS_ITEMS} value={tls} onValueChange={(v) => setTls(v as TlsMode)} disabled={!canEdit}>
                <SelectTrigger id="dashboard-tls" className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {TLS_ITEMS.map((item) => (
                    <SelectItem key={item.value} value={item.value} disabled={item.value === 'tailscale' && !hasTailscale}>
                      {item.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </Field>
          </div>
          <FieldDescription>
            Point the name&apos;s DNS A record at this server first. After saving, the old address stops serving the
            dashboard. To undo from the server: <code className="font-mono text-xs">rig-server dashboard-domain --clear</code>.
          </FieldDescription>
        </FieldGroup>
        {canEdit ? (
          <div className="mt-4 flex justify-end">
            <Button type="submit" disabled={unchanged || save.isPending}>
              {save.isPending ? <Spinner data-icon="inline-start" /> : null}
              Save address
            </Button>
          </div>
        ) : (
          <p className="mt-3 text-xs text-muted-foreground">Only the owner can change this.</p>
        )}
      </form>
    </Panel>
  )
}
