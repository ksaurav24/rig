import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { ChevronDown, Globe, House, Network } from 'lucide-react'
import { useState, type FormEvent } from 'react'

import { api, unwrap, type Schemas } from '@/api/client'
import { meQuery, networkQuery } from '@/api/queries'
import { CopyButton } from '@/components/app/copy-button'
import { Page, Panel } from '@/components/app/page'
import { ErrorState } from '@/components/app/state'
import { Badge } from '@/components/fluid/badge'
import { Field, FieldDescription, FieldGroup, FieldLabel } from '@/components/fluid/field'
import { Input } from '@/components/fluid/input'
import { Select, SelectContent, SelectItem, SelectTrigger } from '@/components/fluid/select'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { hasRole } from '@/lib/nav'
import { cn } from '@/lib/utils'

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
  { value: 'public', title: 'Anywhere', hint: 'Any network that can reach this server.', icon: Globe },
  { value: 'lan', title: 'Local network', hint: 'Private LAN ranges (192.168.x.x, 10.x.x.x).', icon: House },
  { value: 'tailscale', title: 'Tailnet', hint: 'Devices authenticated on your Tailscale network.', icon: Network },
]

/** Docker / container bridge subnets (172.16.0.0/12) often crowd the LAN list. */
function isContainerBridge(ip: string): boolean {
  const m = /^172\.(\d+)\./.exec(ip)
  if (!m) return false
  const second = Number(m[1])
  return second >= 16 && second <= 31
}

export const Route = createFileRoute('/_app/settings/network')({
  staticData: { crumb: 'Network' },
  loader: ({ context }) => void context.queryClient.prefetchQuery(networkQuery),
  component: NetworkPage,
})

function NetworkPage() {
  const network = useQuery(networkQuery)
  const { data: me } = useQuery(meQuery)
  const [showBridges, setShowBridges] = useState(false)

  const lan = network.data?.lan_addresses ?? []
  const primaryLan = lan.filter((ip) => !isContainerBridge(ip))
  const bridgeLan = lan.filter((ip) => isContainerBridge(ip))
  const visibleLan = primaryLan.length > 0 ? (showBridges ? lan : primaryLan) : lan

  return (
    <Page title="Network" className="max-w-3xl">
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
            <dl className="grid gap-x-6 gap-y-3 text-[13px] sm:grid-cols-[11rem_1fr]">
              <dt className="pt-1 text-muted-foreground">Local network</dt>
              <dd className="flex flex-wrap items-center gap-1.5">
                {visibleLan.length ? (
                  visibleLan.map((ip) => <Address key={ip} value={ip} />)
                ) : (
                  <span className="pt-1 text-muted-foreground">None found</span>
                )}
                {primaryLan.length > 0 && bridgeLan.length > 0 ? (
                  <button
                    type="button"
                    onClick={() => setShowBridges((v) => !v)}
                    className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-[12px] text-muted-foreground transition-colors hover:bg-hover hover:text-foreground"
                  >
                    {showBridges ? 'Hide bridges' : `+${bridgeLan.length} bridge`}
                    <ChevronDown className={cn('size-3 transition-transform', showBridges && 'rotate-180')} />
                  </button>
                ) : null}
              </dd>
              <dt className="pt-1 text-muted-foreground">Tailscale</dt>
              <dd className="flex flex-wrap gap-1.5">
                {network.data.tailscale ? (
                  <>
                    {network.data.tailscale.dns_name ? <Address value={network.data.tailscale.dns_name} /> : null}
                    {network.data.tailscale.addresses.map((ip) => (
                      <Address key={ip} value={ip} />
                    ))}
                  </>
                ) : (
                  <span className="pt-1 text-muted-foreground">Not running on this machine</span>
                )}
              </dd>
              <dt className="text-muted-foreground">You are connecting from</dt>
              <dd className="font-mono text-xs text-foreground">{network.data.your_address ?? 'unknown'}</dd>
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
    <span className="inline-flex items-center gap-0.5 rounded-md border border-border/80 bg-surface/60 py-0.5 pr-0.5 pl-2 font-mono text-xs text-foreground">
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
    <Panel title="Dashboard access" description="Control which networks can reach this dashboard (webhooks remain open).">
      <div role="radiogroup" aria-label="Who may open the dashboard" className="grid gap-2.5 sm:grid-cols-3">
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
                'flex flex-col items-start gap-1.5 rounded-lg p-3.5 text-left transition-[box-shadow,background-color] duration-150 outline-none',
                'shadow-[0_0_0_1px_var(--border)] focus-visible:ring-1 focus-visible:ring-[color:var(--focus-ring)] disabled:cursor-not-allowed disabled:opacity-55',
                selected ? 'bg-tint shadow-[0_0_0_1.5px_var(--foreground)]' : 'hover:bg-hover',
              )}
            >
              <span className="flex w-full items-center gap-2 text-[13px] font-medium text-foreground">
                <m.icon className="size-4 text-muted-foreground" aria-hidden />
                {m.title}
                {current === m.value ? (
                  <Badge variant="solid" color="emerald" size="compact" className="ml-auto">
                    Active
                  </Badge>
                ) : null}
              </span>
              <span className="text-[12px] leading-snug text-muted-foreground">
                {unavailable ? 'Tailscale is not running.' : m.hint}
              </span>
            </button>
          )
        })}
      </div>
      <div className="mt-4 flex flex-wrap items-center justify-between gap-3 border-t border-border/70 pt-3.5">
        <p className="text-[12px] text-muted-foreground">
          Locked out? Run <code className="font-mono">rig-server dashboard-access public</code> on the host.
        </p>
        {canEdit ? (
          <Button disabled={choice === current || save.isPending} onClick={() => save.mutate(choice)}>
            {save.isPending ? <Spinner data-icon="inline-start" /> : null}
            Save
          </Button>
        ) : (
          <span className="text-[12px] text-muted-foreground">Owner only</span>
        )}
      </div>
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
            Served at{' '}
            <a
              className="font-mono text-xs text-foreground underline underline-offset-2"
              href={`${current.tls_mode === 'off' ? 'http' : 'https'}://${current.host}/`}
            >
              {current.host}
            </a>
            . Clear to revert to plain HTTP.
          </>
        ) : (
          'Assign a domain for automatic HTTPS and secure cookies.'
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
              <Select value={tls} onValueChange={(v) => setTls(v as TlsMode)} disabled={!canEdit}>
                <SelectTrigger id="dashboard-tls" className="w-full" />
                <SelectContent>
                  {TLS_ITEMS.map((item, i) => (
                    <SelectItem key={item.value} index={i} value={item.value} disabled={item.value === 'tailscale' && !hasTailscale}>
                      {item.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </Field>
          </div>
          <FieldDescription>
            Point the DNS A record at this server first. To clear from SSH:{' '}
            <code className="font-mono text-xs">rig-server dashboard-domain --clear</code>.
          </FieldDescription>
        </FieldGroup>
        <div className="mt-4 flex justify-end border-t border-border/70 pt-3.5">
          {canEdit ? (
            <Button type="submit" disabled={unchanged || save.isPending}>
              {save.isPending ? <Spinner data-icon="inline-start" /> : null}
              Save address
            </Button>
          ) : (
            <span className="text-[12px] text-muted-foreground">Owner only</span>
          )}
        </div>
      </form>
    </Panel>
  )
}
