import { useQuery, useQueryClient } from '@tanstack/react-query'
import {
  Bell,
  Check,
  ChevronDown,
  Copy,
  ExternalLink,
  GitBranch,
  Globe,
  History,
  KeyRound,
  Laptop,
  Lock,
  Plus,
  RefreshCw,
  Save,
  Search,
  Send,
  ShieldCheck,
  Trash2,
  UserPlus,
  Users,
} from 'lucide-react'
import { useEffect, useMemo, useState, type ComponentType, type FormEvent } from 'react'
import { toast } from 'sonner'
import { GithubConnectPanel } from './GithubConnectPanel'
import { Page, Panel } from '@rig/ui/app/page'
import { EmptyState } from '@rig/ui/app/state'
import { Badge } from '@rig/ui/badge'
import { Banner, BannerAction, BannerActions, BannerDescription, BannerTitle } from '@rig/ui/banner'
import { Button } from '@rig/ui/button'
import { Checkbox } from '@rig/ui/checkbox'
import { ConfirmDialog } from '@rig/ui/confirm-dialog'
import { Field, FieldLabel } from '@rig/ui/field'
import { Input } from '@rig/ui/input'
import { Select, SelectContent, SelectItem, SelectTrigger } from '@rig/ui/select'
import { Tabs, TabsItem } from '@rig/ui/tabs'
import { isContainerBridge, relativeTime } from '@/lib/format'
import { desktopApi } from '@/lib/ipc'
import { ACCENT_PRESETS, usePreferences } from '@/lib/preferences'
import type {
  AlertEvent,
  CreatedApiToken,
  MemberWithPassword,
  PairingCode,
  Role,
  ServerView,
} from '@/lib/types'
import { cn } from '@rig/ui/lib/utils'

interface SettingsViewProps {
  server: ServerView
  onRemoveServer: (id: string) => void
}

type SettingsTab = 'general' | 'github' | 'team' | 'devices' | 'alerts' | 'audit'

const ALL_ALERT_EVENTS: { id: AlertEvent; label: string }[] = [
  { id: 'deploy_failed', label: 'Deploy failed' },
  { id: 'app_crashed', label: 'App crashed' },
  { id: 'app_down', label: 'App down' },
  { id: 'disk_low', label: 'Disk low' },
  { id: 'backup_failed', label: 'Backup failed' },
]

const ENV_COLORS: { value: string; label: string; dot: string }[] = [
  { value: 'emerald', label: 'Emerald', dot: 'bg-emerald-500' },
  { value: 'amber', label: 'Amber', dot: 'bg-amber-500' },
  { value: 'red', label: 'Red', dot: 'bg-rose-500' },
  { value: 'blue', label: 'Blue', dot: 'bg-sky-500' },
  { value: 'purple', label: 'Purple', dot: 'bg-violet-500' },
  { value: 'gray', label: 'Neutral', dot: 'bg-zinc-400' },
]

function CopyPill({ value, label }: { value: string; label?: string }) {
  return (
    <span className="inline-flex max-w-full items-center gap-1 rounded-md border border-border/80 bg-surface/70 py-0.5 pr-1 pl-2 font-mono text-[12px] text-foreground">
      <span className="truncate">{value}</span>
      <Button
        type="button"
        variant="ghost"
        size="icon-compact"
        className="size-5 shrink-0"
        aria-label={label ?? `Copy ${value}`}
        onClick={() => {
          void navigator.clipboard.writeText(value)
          toast.success(`Copied ${value}`)
        }}
      >
        <Copy className="size-3" />
      </Button>
    </span>
  )
}

export function SettingsView({ server, onRemoveServer }: SettingsViewProps) {
  const qc = useQueryClient()
  const { resolvedTheme } = usePreferences()
  const [tab, setTab] = useState<SettingsTab>('general')
  const [pairing, setPairing] = useState<PairingCode | null>(null)
  const [creatingPair, setCreatingPair] = useState(false)
  const [showBridges, setShowBridges] = useState(false)

  // Org settings (valid preset accent names + env badge color)
  const [orgName, setOrgName] = useState('')
  const [accentColor, setAccentColor] = useState('emerald')
  const [envLabel, setEnvLabel] = useState('')
  const [envColor, setEnvColor] = useState('emerald')
  const [savingOrg, setSavingOrg] = useState(false)

  // Team
  const [memberName, setMemberName] = useState('')
  const [memberEmail, setMemberEmail] = useState('')
  const [memberRole, setMemberRole] = useState<Role>('developer')
  const [addingMember, setAddingMember] = useState(false)
  const [tempCreds, setTempCreds] = useState<MemberWithPassword | null>(null)

  // API Token creation
  const [tokenName, setTokenName] = useState('')
  const [tokenRole, setTokenRole] = useState<Role>('developer')
  const [tokenExpiryDays, setTokenExpiryDays] = useState('90')
  const [creatingToken, setCreatingToken] = useState(false)
  const [createdToken, setCreatedToken] = useState<CreatedApiToken | null>(null)

  // GitHub repos search
  const [ghRepoFilter, setGhRepoFilter] = useState('')

  // Alerts
  const [alertName, setAlertName] = useState('')
  const [alertUrl, setAlertUrl] = useState('')
  const [alertEvents, setAlertEvents] = useState<AlertEvent[]>(['deploy_failed', 'app_crashed'])
  const [addingAlert, setAddingAlert] = useState(false)
  const [lastSigningSecret, setLastSigningSecret] = useState<string | null>(null)

  const [auditFilter, setAuditFilter] = useState('')
  const [confirmState, setConfirmState] = useState<{
    title: string
    description: string
    target?: string
    confirmLabel: string
    tone: 'danger' | 'warning'
    action: () => void | Promise<void>
  } | null>(null)

  const settingsQuery = useQuery({
    queryKey: ['desktop', server.id, 'settings'],
    queryFn: () => desktopApi.getServerSettings(server.id),
  })

  const data = settingsQuery.data
  const ghApp = data?.github?.app ?? null

  const githubReposQuery = useQuery({
    queryKey: ['desktop', server.id, 'github-repos'],
    queryFn: () => desktopApi.listGithubRepos(server.id),
    enabled: tab === 'github' && Boolean(ghApp),
  })

  useEffect(() => {
    if (!data?.me?.org) return
    setOrgName(data.me.org.name)
    const presetMatch = ACCENT_PRESETS.some((p) => p.value === data.me?.org.accent_color)
    setAccentColor(presetMatch ? data.me.org.accent_color : 'emerald')
    setEnvLabel(data.me.org.env_label ?? '')
    const envMatch = ENV_COLORS.some((c) => c.value === data.me?.org.env_color)
    setEnvColor(envMatch && data.me.org.env_color ? data.me.org.env_color : 'emerald')
  }, [data?.me?.org])

  const filteredAudit = useMemo(() => {
    const entries = data?.audit ?? []
    const q = auditFilter.trim().toLowerCase()
    if (!q) return entries
    return entries.filter(
      (e) =>
        e.actor.toLowerCase().includes(q) ||
        e.action.toLowerCase().includes(q) ||
        (e.target && e.target.toLowerCase().includes(q)) ||
        (e.detail && e.detail.toLowerCase().includes(q)),
    )
  }, [auditFilter, data?.audit])

  const filteredGhRepos = useMemo(() => {
    const repos = githubReposQuery.data ?? []
    const q = ghRepoFilter.trim().toLowerCase()
    if (!q) return repos
    return repos.filter(
      (r) =>
        r.full_name.toLowerCase().includes(q) ||
        r.default_branch.toLowerCase().includes(q),
    )
  }, [ghRepoFilter, githubReposQuery.data])

  const lanAddresses = data?.network?.lan_addresses ?? []
  const primaryLan = lanAddresses.filter((ip) => !isContainerBridge(ip))
  const bridgeLan = lanAddresses.filter((ip) => isContainerBridge(ip))
  const visibleLan = primaryLan.length > 0 ? (showBridges ? lanAddresses : primaryLan) : lanAddresses

  async function saveOrg(e: FormEvent) {
    e.preventDefault()
    setSavingOrg(true)
    try {
      const trimmedEnv = envLabel.trim()
      await desktopApi.updateOrganization(server.id, {
        name: orgName.trim() || undefined,
        accent_color: accentColor,
        env_label: trimmedEnv,
        env_color: trimmedEnv ? envColor : '',
      })
      toast.success('Updated organization settings')
      void settingsQuery.refetch()
    } catch (err) {
      toast.error(String(err))
    } finally {
      setSavingOrg(false)
    }
  }

  async function generateCode() {
    setCreatingPair(true)
    try {
      const p = await desktopApi.createPairingCode(server.id)
      setPairing(p)
    } catch (e) {
      toast.error(String(e))
    } finally {
      setCreatingPair(false)
    }
  }

  async function handleCreateToken(e: FormEvent) {
    e.preventDefault()
    if (!tokenName.trim()) return
    setCreatingToken(true)
    try {
      const days = tokenExpiryDays.trim() ? Number(tokenExpiryDays) : null
      const res = await desktopApi.createToken(server.id, {
        name: tokenName.trim(),
        role: tokenRole,
        expires_in_days: days && Number.isFinite(days) && days > 0 ? days : null,
      })
      setCreatedToken(res)
      setTokenName('')
      toast.success(`Created API token "${res.token.name}"`)
      void settingsQuery.refetch()
    } catch (err) {
      toast.error(String(err))
    } finally {
      setCreatingToken(false)
    }
  }

  async function handleAddMember(e: FormEvent) {
    e.preventDefault()
    if (!memberName.trim() || !memberEmail.trim()) return
    setAddingMember(true)
    try {
      const created = await desktopApi.addMember(server.id, {
        name: memberName.trim(),
        email: memberEmail.trim(),
        role: memberRole,
      })
      setTempCreds(created)
      setMemberName('')
      setMemberEmail('')
      toast.success(`Added ${created.member.name}`)
      void settingsQuery.refetch()
    } catch (err) {
      toast.error(String(err))
    } finally {
      setAddingMember(false)
    }
  }

  async function handleAddAlert(e: FormEvent) {
    e.preventDefault()
    if (!alertName.trim() || !alertUrl.trim() || alertEvents.length === 0) return
    setAddingAlert(true)
    try {
      const res = await desktopApi.createAlertChannel(server.id, {
        name: alertName.trim(),
        url: alertUrl.trim(),
        events: alertEvents,
      })
      setLastSigningSecret(res.signing_secret)
      setAlertName('')
      setAlertUrl('')
      toast.success('Alert channel created')
      void settingsQuery.refetch()
    } catch (err) {
      toast.error(String(err))
    } finally {
      setAddingAlert(false)
    }
  }

  const tabs: {
    id: SettingsTab
    label: string
    icon: ComponentType<{ size?: number; strokeWidth?: number; className?: string }>
  }[] = [
    { id: 'general', label: 'General & Network', icon: Globe },
    { id: 'github', label: ghApp ? `GitHub (${ghApp.owner})` : 'GitHub', icon: GitBranch },
    { id: 'team', label: `Team (${data?.members.length ?? 0})`, icon: Users },
    {
      id: 'devices',
      label: `Devices & Tokens (${(data?.devices.length ?? 0) + (data?.tokens.length ?? 0)})`,
      icon: Laptop,
    },
    { id: 'alerts', label: `Alerts (${data?.alerts.length ?? 0})`, icon: Bell },
    { id: 'audit', label: 'Audit Log', icon: History },
  ]
  const tabIndex = Math.max(
    0,
    tabs.findIndex((t) => t.id === tab),
  )

  const webhookFullUrl = data?.github?.webhook_url
    ? `${server.url.replace(/\/$/, '')}${data.github.webhook_url}`
    : null

  return (
    <Page
      title="Server Settings"
      description={
        <>
          Manage organization branding, network addresses, GitHub integration, team members, paired
          devices, webhooks, and audit logs on{' '}
          <span className="font-medium text-foreground">{server.name}</span>.
        </>
      }
      actions={
        <Button
          type="button"
          variant="tertiary"
          leadingIcon={RefreshCw}
          loading={settingsQuery.isFetching}
          onClick={() => void settingsQuery.refetch()}
        >
          Refresh
        </Button>
      }
    >
      {/* Fluid Sub-navigation Tabs */}
      <div className="border-b border-border/80 pb-2.5">
        <Tabs
          selectedIndex={tabIndex}
          onSelect={(idx) => {
            const target = tabs[idx]
            if (target) setTab(target.id)
          }}
        >
          {tabs.map((t, idx) => (
            <TabsItem key={t.id} index={idx} label={t.label} icon={t.icon} />
          ))}
        </Tabs>
      </div>

      {/* GENERAL & NETWORK */}
      {tab === 'general' && (
        <div className="space-y-5">
          <Panel title="Organization & Branding">
            <form onSubmit={saveOrg} className="space-y-5">
              <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
                <Field>
                  <FieldLabel htmlFor="org-name">Organization Name</FieldLabel>
                  <Input
                    id="org-name"
                    type="text"
                    value={orgName}
                    onChange={(e) => setOrgName(e.target.value)}
                    placeholder="My Homelab"
                  />
                </Field>
                <Field>
                  <FieldLabel htmlFor="org-env">Environment Badge</FieldLabel>
                  <Input
                    id="org-env"
                    type="text"
                    value={envLabel}
                    onChange={(e) => setEnvLabel(e.target.value)}
                    placeholder="production, staging, homelab…"
                    maxLength={24}
                  />
                </Field>
              </div>

              {envLabel.trim() && (
                <Field>
                  <FieldLabel>Environment Badge Color</FieldLabel>
                  <div className="flex flex-wrap items-center gap-1.5">
                    {ENV_COLORS.map((c) => {
                      const active = envColor === c.value
                      return (
                        <button
                          key={c.value}
                          type="button"
                          onClick={() => setEnvColor(c.value)}
                          className={cn(
                            'inline-flex h-7 items-center gap-1.5 rounded-full border px-2.5 text-[12px] transition-colors',
                            active
                              ? 'border-primary bg-primary/12 font-medium text-foreground'
                              : 'border-border bg-surface text-muted-foreground hover:text-foreground',
                          )}
                        >
                          <span className={cn('size-2 rounded-full', c.dot)} />
                          {c.label}
                        </button>
                      )
                    })}
                  </div>
                </Field>
              )}

              <Field>
                <FieldLabel>Organization Accent Color</FieldLabel>
                <div className="flex flex-wrap gap-2">
                  {ACCENT_PRESETS.map((preset) => {
                    const active = accentColor === preset.value
                    const swatch =
                      resolvedTheme === 'dark' ? preset.swatchDark : preset.swatchLight
                    return (
                      <button
                        key={preset.value}
                        type="button"
                        onClick={() => setAccentColor(preset.value)}
                        className={cn(
                          'inline-flex items-center gap-2 rounded-full border px-3 py-1.5 text-[12.5px] font-medium transition-colors duration-80',
                          active
                            ? 'border-primary bg-primary/12 text-foreground'
                            : 'border-border bg-surface text-muted-foreground hover:bg-hover hover:text-foreground',
                        )}
                      >
                        <span
                          className="flex size-3.5 items-center justify-center rounded-full"
                          style={{ backgroundColor: swatch }}
                        >
                          {active && (
                            <Check className="size-2.5 text-primary-foreground" strokeWidth={2.5} />
                          )}
                        </span>
                        {preset.label}
                      </button>
                    )
                  })}
                </div>
              </Field>

              <div className="flex flex-wrap items-center justify-between gap-3 border-t border-border/80 pt-4">
                <span className="text-[13px] text-muted-foreground">
                  Signed in as{' '}
                  <span className="font-mono text-foreground">
                    {data?.me?.user.email ?? server.user_email ?? '—'}
                  </span>{' '}
                  ({data?.me?.role ?? 'member'})
                </span>
                <Button type="submit" variant="primary" leadingIcon={Save} loading={savingOrg}>
                  Save Organization
                </Button>
              </div>
            </form>
          </Panel>

          <Panel
            title={
              <>
                <ShieldCheck className="size-4 text-primary" />
                Network, Connection & Capabilities
              </>
            }
          >
            <dl className="grid grid-cols-1 gap-4 text-[13.5px] sm:grid-cols-2">
              <div>
                <dt className="text-[12.5px] text-muted-foreground">Connected Endpoint</dt>
                <dd className="mt-1.5">
                  <CopyPill value={server.url} label="Copy connected endpoint" />
                </dd>
              </div>
              <div>
                <dt className="text-[12.5px] text-muted-foreground">Server Version / Mode</dt>
                <dd className="mt-1.5 font-mono text-[13px] text-foreground">
                  {server.meta
                    ? `v${server.meta.server_version} (API v${server.meta.api_version}) · ${server.meta.mode ?? 'full'} · ${server.meta.access ?? data?.network?.dashboard_access ?? '—'}`
                    : 'Unknown'}
                </dd>
              </div>

              <div>
                <dt className="text-[12.5px] text-muted-foreground">Public IP</dt>
                <dd className="mt-1.5">
                  {data?.network?.public_ip ? (
                    <CopyPill value={data.network.public_ip} label="Copy server public IP" />
                  ) : (
                    <span className="font-mono text-[13px] text-muted-foreground">
                      {settingsQuery.isPending ? 'Detecting…' : 'Not detected'}
                    </span>
                  )}
                </dd>
              </div>

              <div>
                <dt className="text-[12.5px] text-muted-foreground">Your Connection Address</dt>
                <dd className="mt-1.5 font-mono text-[13px] text-foreground">
                  {data?.network?.your_address ?? '—'}
                </dd>
              </div>

              {data?.network && (
                <>
                  <div>
                    <dt className="text-[12.5px] text-muted-foreground">LAN Addresses</dt>
                    <dd className="mt-1.5 flex flex-wrap items-center gap-1.5">
                      {visibleLan.length > 0 ? (
                        visibleLan.map((ip) => <CopyPill key={ip} value={ip} />)
                      ) : (
                        <span className="font-mono text-[13px] text-muted-foreground">—</span>
                      )}
                      {primaryLan.length > 0 && bridgeLan.length > 0 && (
                        <button
                          type="button"
                          onClick={() => setShowBridges((v) => !v)}
                          className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-[12px] text-muted-foreground transition-colors hover:bg-hover hover:text-foreground"
                        >
                          {showBridges ? 'Hide bridges' : `+${bridgeLan.length} bridge`}
                          <ChevronDown
                            className={cn('size-3 transition-transform', showBridges && 'rotate-180')}
                          />
                        </button>
                      )}
                    </dd>
                  </div>

                  <div>
                    <dt className="text-[12.5px] text-muted-foreground">Tailscale</dt>
                    <dd className="mt-1.5 flex flex-wrap items-center gap-1.5">
                      {data.network.tailscale ? (
                        <>
                          {data.network.tailscale.dns_name && (
                            <CopyPill value={data.network.tailscale.dns_name} />
                          )}
                          {data.network.tailscale.addresses.map((ip) => (
                            <CopyPill key={ip} value={ip} />
                          ))}
                        </>
                      ) : (
                        <span className="font-mono text-[13px] text-muted-foreground">
                          Not detected
                        </span>
                      )}
                    </dd>
                  </div>

                  {data.network.dashboard_domain && (
                    <div>
                      <dt className="text-[12.5px] text-muted-foreground">Dashboard Domain</dt>
                      <dd className="mt-1.5 flex flex-wrap items-center gap-2">
                        <CopyPill value={data.network.dashboard_domain.host} />
                        <Badge variant="solid" color="gray" size="compact" className="font-mono">
                          TLS: {data.network.dashboard_domain.tls_mode}
                        </Badge>
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon-compact"
                          aria-label="Open dashboard in browser"
                          onClick={() => {
                            const d = data.network!.dashboard_domain!
                            const scheme = d.tls_mode === 'off' ? 'http' : 'https'
                            void desktopApi.openExternalUrl(`${scheme}://${d.host}/`)
                          }}
                        >
                          <ExternalLink />
                        </Button>
                      </dd>
                    </div>
                  )}
                </>
              )}

              <div>
                <dt className="text-[12.5px] text-muted-foreground">TLS Pinning</dt>
                <dd className="mt-1.5">
                  {server.pinned_tls_sha256 ? (
                    <CopyPill value={server.pinned_tls_sha256} label="Copy TLS SHA-256 fingerprint" />
                  ) : (
                    <span className="font-mono text-[12.5px] text-foreground">
                      Standard Public WebPKI Roots
                    </span>
                  )}
                </dd>
              </div>

              <div className="sm:col-span-2">
                <dt className="text-[12.5px] text-muted-foreground">Capabilities</dt>
                <dd className="mt-1.5 flex flex-wrap gap-1.5">
                  {(server.meta?.capabilities ?? []).length === 0 ? (
                    <span className="text-[12.5px] text-muted-foreground">—</span>
                  ) : (
                    (server.meta?.capabilities ?? []).map((c) => (
                      <Badge key={c} variant="solid" color="gray" size="compact" className="font-mono">
                        {c}
                      </Badge>
                    ))
                  )}
                </dd>
              </div>
            </dl>
          </Panel>

          <Panel
            tone="danger"
            title="Disconnect from Rig Desktop"
            description="Removes this server from the switcher and deletes its token from your OS keychain."
            actions={
              <Button
                type="button"
                variant="destructive"
                onClick={() =>
                  setConfirmState({
                    title: 'Disconnect Server',
                    description:
                      'Removes this server from Rig Desktop and deletes its device token from your OS keychain. Running apps on the server are unaffected.',
                    target: `${server.name} (${server.url})`,
                    confirmLabel: 'Remove Server',
                    tone: 'danger',
                    action: () => onRemoveServer(server.id),
                  })
                }
              >
                Remove Server
              </Button>
            }
          >
            <p className="text-[13px] text-muted-foreground">
              The server itself and its running workloads will not be affected.
            </p>
          </Panel>
        </div>
      )}

      {/* GITHUB INTEGRATION */}
      {tab === 'github' && (
        <div className="space-y-5">
          {!ghApp ? (
            <Panel
              title="Connect GitHub App"
              description="One private GitHub App shared across all projects on this server."
            >
              <GithubConnectPanel
                serverId={server.id}
                onConnected={(status) => {
                  qc.setQueryData(['desktop', server.id, 'github-status'], status)
                  void settingsQuery.refetch()
                  void qc.invalidateQueries({ queryKey: ['desktop', server.id, 'github-repos'] })
                }}
              />
            </Panel>
          ) : (
            <>
              <Panel
                title={
                  <>
                    <GitBranch className="size-4 text-primary" />
                    {ghApp.name}
                  </>
                }
                description={`Owned by ${ghApp.owner} · connected ${relativeTime(ghApp.created_at)}`}
                actions={
                  <div className="flex flex-wrap items-center gap-2">
                    <Button
                      type="button"
                      variant="primary"
                      size="compact"
                      trailingIcon={ExternalLink}
                      onClick={() => void desktopApi.openExternalUrl(ghApp.install_url)}
                    >
                      Install on Repositories
                    </Button>
                    <Button
                      type="button"
                      variant="tertiary"
                      size="compact"
                      trailingIcon={ExternalLink}
                      onClick={() => void desktopApi.openExternalUrl(ghApp.html_url)}
                    >
                      App Settings
                    </Button>
                    <Button
                      type="button"
                      variant="destructive"
                      size="compact"
                      leadingIcon={Trash2}
                      onClick={() =>
                        setConfirmState({
                          title: 'Disconnect GitHub App',
                          description:
                            'Removes the GitHub App keys from this Rig server. Private repositories will stop cloning and push-to-deploy webhooks will stop until reconnected. Delete the App on GitHub too if no longer needed.',
                          target: `${ghApp.name} (${ghApp.owner})`,
                          confirmLabel: 'Disconnect GitHub',
                          tone: 'danger',
                          action: async () => {
                            try {
                              await desktopApi.disconnectGithub(server.id)
                              toast.success('Disconnected GitHub App')
                              void settingsQuery.refetch()
                              void qc.invalidateQueries({
                                queryKey: ['desktop', server.id, 'github-status'],
                              })
                              void qc.invalidateQueries({
                                queryKey: ['desktop', server.id, 'github-repos'],
                              })
                            } catch (e) {
                              toast.error(String(e))
                            }
                          },
                        })
                      }
                    >
                      Disconnect
                    </Button>
                  </div>
                }
              >
                <p className="text-[13px] text-muted-foreground">
                  Private repositories clone using short-lived installation tokens. Pick any
                  installed repository and branch directly when deploying a new app.
                </p>
              </Panel>

              <Panel
                title={`Installed Repositories (${githubReposQuery.data?.length ?? 0})`}
                description="Repositories this GitHub App is authorized to read and deploy."
                actions={
                  <div className="flex items-center gap-2">
                    <div className="relative">
                      <Search className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground" />
                      <Input
                        size="compact"
                        type="text"
                        value={ghRepoFilter}
                        onChange={(e) => setGhRepoFilter(e.target.value)}
                        placeholder="Filter repos…"
                        className="w-48 pl-8"
                      />
                    </div>
                    <Button
                      type="button"
                      variant="tertiary"
                      size="compact"
                      leadingIcon={RefreshCw}
                      loading={githubReposQuery.isFetching}
                      onClick={() => void githubReposQuery.refetch()}
                    >
                      Refresh
                    </Button>
                  </div>
                }
                flush
              >
                {githubReposQuery.isPending ? (
                  <div className="px-5 py-8 text-center text-[13px] text-muted-foreground">
                    Loading repositories from GitHub…
                  </div>
                ) : githubReposQuery.isError ? (
                  <div className="p-4">
                    <Banner status="error" contrast="high" size="compact">
                      <BannerDescription>{String(githubReposQuery.error)}</BannerDescription>
                    </Banner>
                  </div>
                ) : filteredGhRepos.length === 0 ? (
                  <div className="p-6">
                    <EmptyState
                      icon={GitBranch}
                      title={
                        (githubReposQuery.data?.length ?? 0) === 0
                          ? 'No repositories installed yet'
                          : 'No matching repositories'
                      }
                      action={
                        (githubReposQuery.data?.length ?? 0) === 0 ? (
                          <Button
                            type="button"
                            variant="primary"
                            trailingIcon={ExternalLink}
                            onClick={() => void desktopApi.openExternalUrl(ghApp.install_url)}
                          >
                            Choose Repositories on GitHub
                          </Button>
                        ) : undefined
                      }
                    >
                      {(githubReposQuery.data?.length ?? 0) === 0
                        ? 'Install the App on the repositories you want to deploy, then click Refresh.'
                        : 'Try a different search term.'}
                    </EmptyState>
                  </div>
                ) : (
                  <div className="divide-y divide-border/80 text-[13.5px]">
                    {filteredGhRepos.map((repo) => (
                      <div
                        key={repo.full_name}
                        className="flex items-center justify-between gap-3 px-5 py-3"
                      >
                        <div className="flex min-w-0 items-center gap-2.5">
                          {repo.private ? (
                            <Lock className="size-3.5 shrink-0 text-muted-foreground" />
                          ) : (
                            <Globe className="size-3.5 shrink-0 text-muted-foreground/70" />
                          )}
                          <span className="truncate font-medium text-foreground">
                            {repo.full_name}
                          </span>
                          <Badge variant="solid" color="gray" size="compact" className="font-mono">
                            {repo.default_branch}
                          </Badge>
                        </div>
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon-compact"
                          title="Copy clone URL"
                          aria-label={`Copy clone URL for ${repo.full_name}`}
                          onClick={() => {
                            void navigator.clipboard.writeText(repo.clone_url)
                            toast.success(`Copied ${repo.clone_url}`)
                          }}
                        >
                          <Copy />
                        </Button>
                      </div>
                    ))}
                  </div>
                )}
              </Panel>
            </>
          )}

          {data?.github && (
            <Banner status="info" contrast="low">
              <BannerTitle>Webhooks & Private Network Polling</BannerTitle>
              <BannerDescription>
                {data.github.poll_interval > 0
                  ? `Rig polls deploy-on-push branches every ${data.github.poll_interval}s when GitHub webhooks cannot reach this server.`
                  : 'Branch polling is disabled on this server (--poll-interval 0).'}
                {webhookFullUrl && (
                  <div className="mt-2">
                    <CopyPill value={webhookFullUrl} label="Copy GitHub webhook URL" />
                  </div>
                )}
              </BannerDescription>
            </Banner>
          )}
        </div>
      )}

      {/* TEAM & MEMBERS */}
      {tab === 'team' && (
        <div className="space-y-5">
          <Panel>
            <form onSubmit={handleAddMember} className="flex flex-wrap items-center gap-2.5">
              <Input
                type="text"
                required
                value={memberName}
                onChange={(e) => setMemberName(e.target.value)}
                placeholder="Full name"
                className="w-48"
              />
              <Input
                type="email"
                required
                value={memberEmail}
                onChange={(e) => setMemberEmail(e.target.value)}
                placeholder="member@company.com"
                className="min-w-[220px] flex-1"
              />
              <Select value={memberRole} onValueChange={(v) => setMemberRole(v as Role)}>
                <SelectTrigger className="w-[150px]" placeholder="Role" />
                <SelectContent>
                  <SelectItem index={0} value="viewer">
                    Viewer
                  </SelectItem>
                  <SelectItem index={1} value="developer">
                    Developer
                  </SelectItem>
                  <SelectItem index={2} value="admin">
                    Admin
                  </SelectItem>
                  <SelectItem index={3} value="owner">
                    Owner
                  </SelectItem>
                </SelectContent>
              </Select>
              <Button
                type="submit"
                variant="primary"
                leadingIcon={UserPlus}
                loading={addingMember}
              >
                Add Member
              </Button>
            </form>
          </Panel>

          {tempCreds && (
            <Banner status="success" contrast="high">
              <BannerTitle>Temporary password for {tempCreds.member.email}</BannerTitle>
              <BannerDescription className="font-mono text-[14px]">
                {tempCreds.temporary_password}
              </BannerDescription>
              <BannerActions>
                <BannerAction
                  variant="secondary"
                  onClick={() => {
                    void navigator.clipboard.writeText(tempCreds.temporary_password)
                    toast.success('Copied temporary password')
                  }}
                >
                  Copy Password
                </BannerAction>
              </BannerActions>
            </Banner>
          )}

          <Panel flush>
            <div className="divide-y divide-border/80 text-[13.5px]">
              {(data?.members ?? []).length === 0 ? (
                <div className="px-5 py-10 text-center text-muted-foreground">
                  No members returned (requires admin/owner role).
                </div>
              ) : (
                (data?.members ?? []).map((m) => (
                  <div
                    key={m.user_id}
                    className="flex flex-wrap items-center justify-between gap-3 px-5 py-3.5"
                  >
                    <div>
                      <div className="font-medium text-foreground">{m.name}</div>
                      <div className="mt-0.5 font-mono text-[12px] text-muted-foreground">
                        {m.email} · last login {relativeTime(m.last_login_at)}
                      </div>
                    </div>
                    <div className="flex flex-wrap items-center gap-2">
                      <Select
                        size="compact"
                        value={m.role}
                        onValueChange={async (v) => {
                          const role = v as Role
                          try {
                            await desktopApi.updateMember(server.id, m.user_id, { role })
                            toast.success(`Updated ${m.name} to ${role}`)
                            void settingsQuery.refetch()
                          } catch (err) {
                            toast.error(String(err))
                          }
                        }}
                      >
                        <SelectTrigger className="w-[130px]" />
                        <SelectContent>
                          <SelectItem index={0} value="viewer">
                            Viewer
                          </SelectItem>
                          <SelectItem index={1} value="developer">
                            Developer
                          </SelectItem>
                          <SelectItem index={2} value="admin">
                            Admin
                          </SelectItem>
                          <SelectItem index={3} value="owner">
                            Owner
                          </SelectItem>
                        </SelectContent>
                      </Select>
                      <Button
                        type="button"
                        variant="tertiary"
                        size="compact"
                        onClick={() =>
                          setConfirmState({
                            title: 'Reset Member Password',
                            description:
                              'Generates a new one-time temporary password and invalidates this member’s current password immediately.',
                            target: `${m.name} (${m.email})`,
                            confirmLabel: 'Reset Password',
                            tone: 'warning',
                            action: async () => {
                              try {
                                const res = await desktopApi.resetMemberPassword(
                                  server.id,
                                  m.user_id,
                                )
                                setTempCreds(res)
                                toast.success(`Reset password for ${m.name}`)
                              } catch (err) {
                                toast.error(String(err))
                              }
                            },
                          })
                        }
                      >
                        Reset Password
                      </Button>
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon-compact"
                        aria-label={`Remove ${m.name}`}
                        onClick={() =>
                          setConfirmState({
                            title: 'Remove Team Member',
                            description:
                              'Revokes this user’s access to the Rig server immediately.',
                            target: `${m.name} (${m.email})`,
                            confirmLabel: 'Remove Member',
                            tone: 'danger',
                            action: async () => {
                              try {
                                await desktopApi.removeMember(server.id, m.user_id)
                                toast.success(`Removed ${m.name}`)
                                void settingsQuery.refetch()
                              } catch (err) {
                                toast.error(String(err))
                              }
                            },
                          })
                        }
                        className="text-muted-foreground hover:text-fail"
                      >
                        <Trash2 />
                      </Button>
                    </div>
                  </div>
                ))
              )}
            </div>
          </Panel>
        </div>
      )}

      {/* DEVICES & API TOKENS */}
      {tab === 'devices' && (
        <div className="space-y-5">
          <Panel
            title={
              <>
                <Laptop className="size-4 text-primary" />
                Paired Devices ({data?.devices.length ?? 0})
              </>
            }
            actions={
              <Button
                type="button"
                variant="primary"
                size="compact"
                leadingIcon={KeyRound}
                loading={creatingPair}
                onClick={generateCode}
              >
                Pair Another Device
              </Button>
            }
            flush
          >
            {pairing && (
              <div className="border-b border-border/80 p-4">
                <Banner status="success" contrast="high">
                  <BannerTitle>One-time pairing code (single use, valid 5 min)</BannerTitle>
                  <BannerDescription className="font-mono text-lg font-bold tracking-[0.2em]">
                    {pairing.code}
                  </BannerDescription>
                  <BannerActions>
                    <BannerAction
                      variant="secondary"
                      onClick={() => {
                        void navigator.clipboard.writeText(pairing.code)
                        toast.success('Copied pairing code')
                      }}
                    >
                      Copy Code
                    </BannerAction>
                  </BannerActions>
                </Banner>
              </div>
            )}

            <div className="divide-y divide-border/80 text-[13.5px]">
              {(data?.devices ?? []).length === 0 ? (
                <div className="px-5 py-8 text-center text-muted-foreground">
                  No paired devices found.
                </div>
              ) : (
                (data?.devices ?? []).map((dev) => (
                  <div
                    key={dev.id}
                    className="flex items-center justify-between gap-3 px-5 py-3.5"
                  >
                    <div className="min-w-0">
                      <div className="font-medium text-foreground">{dev.name}</div>
                      <div className="mt-0.5 font-mono text-[12px] text-muted-foreground">
                        {dev.prefix}… · {dev.role} · IP {dev.last_ip ?? '—'} · last seen{' '}
                        {relativeTime(dev.last_used_at)}
                      </div>
                    </div>
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon-compact"
                      aria-label={`Revoke ${dev.name}`}
                      onClick={() =>
                        setConfirmState({
                          title: 'Revoke Paired Device',
                          description:
                            'Immediately disconnects this device and terminates its active log/event streams.',
                          target: `${dev.name} (${dev.prefix}…)`,
                          confirmLabel: 'Revoke Device',
                          tone: 'danger',
                          action: async () => {
                            try {
                              await desktopApi.revokeDevice(server.id, dev.id)
                              toast.success(`Revoked ${dev.name}`)
                              void settingsQuery.refetch()
                            } catch (e) {
                              toast.error(String(e))
                            }
                          },
                        })
                      }
                      title="Revoke device token"
                      className="text-muted-foreground hover:text-fail"
                    >
                      <Trash2 />
                    </Button>
                  </div>
                ))
              )}
            </div>
          </Panel>

          <Panel title="Create API / CLI Token">
            <form onSubmit={handleCreateToken} className="flex flex-wrap items-center gap-2.5">
              <Input
                type="text"
                required
                value={tokenName}
                onChange={(e) => setTokenName(e.target.value)}
                placeholder="Token name (e.g. GitHub Actions CI, Laptop CLI)"
                className="min-w-[220px] flex-1"
              />
              <Select value={tokenRole} onValueChange={(v) => setTokenRole(v as Role)}>
                <SelectTrigger className="w-[140px]" placeholder="Role" />
                <SelectContent>
                  <SelectItem index={0} value="viewer">
                    Viewer
                  </SelectItem>
                  <SelectItem index={1} value="developer">
                    Developer
                  </SelectItem>
                  <SelectItem index={2} value="admin">
                    Admin
                  </SelectItem>
                  <SelectItem index={3} value="owner">
                    Owner
                  </SelectItem>
                </SelectContent>
              </Select>
              <Input
                type="number"
                min={1}
                max={3650}
                value={tokenExpiryDays}
                onChange={(e) => setTokenExpiryDays(e.target.value)}
                placeholder="Days (empty = never)"
                className="w-36 font-mono"
                title="Expiry in days (leave blank for non-expiring)"
              />
              <Button
                type="submit"
                variant="primary"
                leadingIcon={Plus}
                loading={creatingToken}
                disabled={!tokenName.trim()}
              >
                Create Token
              </Button>
            </form>

            {createdToken && (
              <div className="mt-4">
                <Banner status="success" contrast="high">
                  <BannerTitle>
                    Token created: {createdToken.token.name} (shown once)
                  </BannerTitle>
                  <BannerDescription className="break-all font-mono text-[13px]">
                    {createdToken.secret}
                  </BannerDescription>
                  <BannerActions>
                    <BannerAction
                      variant="secondary"
                      leadingIcon={Copy}
                      onClick={() => {
                        void navigator.clipboard.writeText(createdToken.secret)
                        toast.success('Copied API token secret')
                      }}
                    >
                      Copy Secret
                    </BannerAction>
                  </BannerActions>
                </Banner>
              </div>
            )}
          </Panel>

          <Panel
            title={
              <>
                <KeyRound className="size-4 text-busy" />
                API & CLI Tokens ({data?.tokens.length ?? 0})
              </>
            }
            flush
          >
            <div className="divide-y divide-border/80 text-[13.5px]">
              {(data?.tokens ?? []).length === 0 ? (
                <div className="px-5 py-8 text-center text-muted-foreground">
                  No API tokens found.
                </div>
              ) : (
                (data?.tokens ?? []).map((tok) => (
                  <div
                    key={tok.id}
                    className="flex items-center justify-between gap-3 px-5 py-3.5"
                  >
                    <div className="min-w-0">
                      <div className="flex items-center gap-2 font-medium text-foreground">
                        <span>{tok.name}</span>
                        {tok.kind && (
                          <Badge variant="solid" color="gray" size="compact" className="font-mono">
                            {tok.kind}
                          </Badge>
                        )}
                      </div>
                      <div className="mt-0.5 font-mono text-[12px] text-muted-foreground">
                        {tok.prefix}… · {tok.role} · created {relativeTime(tok.created_at)}
                      </div>
                    </div>
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon-compact"
                      aria-label={`Revoke token ${tok.name}`}
                      onClick={() =>
                        setConfirmState({
                          title: 'Revoke API Token',
                          description:
                            'Permanently invalidates this token. Any CLI, MCP server, or CI workflow using it will fail to authenticate.',
                          target: `${tok.name} (${tok.prefix}…)`,
                          confirmLabel: 'Revoke Token',
                          tone: 'danger',
                          action: async () => {
                            try {
                              await desktopApi.revokeToken(server.id, tok.id)
                              toast.success(`Revoked token ${tok.name}`)
                              void settingsQuery.refetch()
                            } catch (e) {
                              toast.error(String(e))
                            }
                          },
                        })
                      }
                      title="Revoke token"
                      className="text-muted-foreground hover:text-fail"
                    >
                      <Trash2 />
                    </Button>
                  </div>
                ))
              )}
            </div>
          </Panel>
        </div>
      )}

      {/* ALERTS */}
      {tab === 'alerts' && (
        <div className="space-y-5">
          <Panel title="Add Webhook Alert Channel">
            <form onSubmit={handleAddAlert} className="space-y-4">
              <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
                <Input
                  type="text"
                  required
                  value={alertName}
                  onChange={(e) => setAlertName(e.target.value)}
                  placeholder="Channel name (e.g. Discord Ops)"
                />
                <Input
                  type="url"
                  required
                  value={alertUrl}
                  onChange={(e) => setAlertUrl(e.target.value)}
                  placeholder="https://discord.com/api/webhooks/..."
                  className="font-mono sm:col-span-2"
                />
              </div>
              <div className="flex flex-wrap items-center justify-between gap-3">
                <div className="flex flex-wrap items-center gap-4">
                  {ALL_ALERT_EVENTS.map((ev) => {
                    const checked = alertEvents.includes(ev.id)
                    return (
                      <label
                        key={ev.id}
                        className="inline-flex cursor-pointer items-center gap-2 text-[13.5px] text-muted-foreground hover:text-foreground"
                      >
                        <Checkbox
                          checked={checked}
                          onCheckedChange={(isChecked) => {
                            setAlertEvents((prev) =>
                              isChecked ? [...prev, ev.id] : prev.filter((x) => x !== ev.id),
                            )
                          }}
                        />
                        {ev.label}
                      </label>
                    )
                  })}
                </div>
                <Button
                  type="submit"
                  variant="primary"
                  leadingIcon={Plus}
                  loading={addingAlert}
                  disabled={alertEvents.length === 0}
                >
                  Add Channel
                </Button>
              </div>
            </form>
          </Panel>

          {lastSigningSecret && (
            <Banner status="success" contrast="high">
              <BannerTitle>Webhook signing secret</BannerTitle>
              <BannerDescription className="font-mono">{lastSigningSecret}</BannerDescription>
              <BannerActions>
                <BannerAction
                  variant="secondary"
                  leadingIcon={Copy}
                  onClick={() => {
                    void navigator.clipboard.writeText(lastSigningSecret)
                    toast.success('Copied signing secret')
                  }}
                >
                  Copy
                </BannerAction>
              </BannerActions>
            </Banner>
          )}

          <Panel flush>
            <div className="divide-y divide-border/80 text-[13.5px]">
              {(data?.alerts ?? []).length === 0 ? (
                <div className="px-5 py-10 text-center text-muted-foreground">
                  No alert channels configured yet.
                </div>
              ) : (
                (data?.alerts ?? []).map((ch) => (
                  <div
                    key={ch.id}
                    className="flex flex-wrap items-center justify-between gap-3 px-5 py-3.5"
                  >
                    <div>
                      <div className="flex flex-wrap items-center gap-2 font-medium text-foreground">
                        <span>{ch.name}</span>
                        <span className="font-mono text-[12px] text-muted-foreground">
                          {ch.url_host}
                        </span>
                      </div>
                      <div className="mt-1.5 flex flex-wrap gap-1.5">
                        {ch.events.map((ev) => (
                          <Badge key={ev} variant="solid" color="gray" size="compact" className="font-mono">
                            {ev}
                          </Badge>
                        ))}
                      </div>
                      {ch.last_error && (
                        <div className="mt-1.5 text-[12.5px] text-fail">{ch.last_error}</div>
                      )}
                    </div>
                    <div className="flex items-center gap-2">
                      <Button
                        type="button"
                        variant="tertiary"
                        size="compact"
                        leadingIcon={Send}
                        onClick={async () => {
                          try {
                            const res = await desktopApi.testAlertChannel(server.id, ch.id)
                            if (res.ok) toast.success(`Test sent to ${ch.name}`)
                            else toast.error(res.error ?? 'Test failed')
                          } catch (err) {
                            toast.error(String(err))
                          }
                        }}
                      >
                        Test
                      </Button>
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon-compact"
                        aria-label={`Delete ${ch.name}`}
                        onClick={() =>
                          setConfirmState({
                            title: 'Delete Alert Channel',
                            description:
                              'Removes this webhook alert channel. Future deploy failures and crash alerts will no longer be delivered here.',
                            target: `${ch.name} (${ch.url_host})`,
                            confirmLabel: 'Delete Channel',
                            tone: 'danger',
                            action: async () => {
                              try {
                                await desktopApi.deleteAlertChannel(server.id, ch.id)
                                toast.success(`Deleted ${ch.name}`)
                                void settingsQuery.refetch()
                              } catch (err) {
                                toast.error(String(err))
                              }
                            },
                          })
                        }
                        className="text-muted-foreground hover:text-fail"
                      >
                        <Trash2 />
                      </Button>
                    </div>
                  </div>
                ))
              )}
            </div>
          </Panel>
        </div>
      )}

      {/* AUDIT LOG */}
      {tab === 'audit' && (
        <div className="space-y-4">
          <div className="relative">
            <Search className="pointer-events-none absolute top-1/2 left-3.5 size-4 -translate-y-1/2 text-muted-foreground" />
            <Input
              type="text"
              value={auditFilter}
              onChange={(e) => setAuditFilter(e.target.value)}
              placeholder="Filter audit log by actor, action, target, or detail…"
              className="pl-10"
            />
          </div>
          <Panel flush>
            <div className="divide-y divide-border/80 text-[13.5px]">
              {filteredAudit.length === 0 ? (
                <div className="px-5 py-10 text-center text-muted-foreground">
                  No audit entries found.
                </div>
              ) : (
                filteredAudit.map((entry) => (
                  <div
                    key={entry.id}
                    className="flex flex-wrap items-center justify-between gap-2 px-5 py-3"
                  >
                    <div className="min-w-0">
                      <div className="flex flex-wrap items-center gap-2">
                        <span className="font-mono font-semibold text-primary">{entry.action}</span>
                        <span className="text-foreground">{entry.actor}</span>
                        {entry.target && (
                          <span className="font-mono text-[12px] text-muted-foreground">
                            {entry.target}
                          </span>
                        )}
                      </div>
                      {entry.detail && (
                        <div className="mt-0.5 truncate text-[12.5px] text-muted-foreground">
                          {entry.detail}
                        </div>
                      )}
                    </div>
                    <div className="shrink-0 font-mono text-[11.5px] text-muted-foreground tabular-nums">
                      {entry.ip ? `${entry.ip} · ` : ''}
                      {relativeTime(entry.created_at)}
                    </div>
                  </div>
                ))
              )}
            </div>
          </Panel>
        </div>
      )}

      <ConfirmDialog
        open={Boolean(confirmState)}
        onOpenChange={(open) => {
          if (!open) setConfirmState(null)
        }}
        title={confirmState?.title ?? ''}
        description={confirmState?.description ?? ''}
        target={confirmState?.target}
        confirmLabel={confirmState?.confirmLabel}
        tone={confirmState?.tone}
        onConfirm={async () => {
          await confirmState?.action()
        }}
      />
    </Page>
  )
}
