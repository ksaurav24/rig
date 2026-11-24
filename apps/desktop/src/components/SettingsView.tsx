import { useQuery } from '@tanstack/react-query'
import {
  Bell,
  Copy,
  Globe,
  History,
  KeyRound,
  Laptop,
  Palette,
  Plus,
  RefreshCw,
  Save,
  Send,
  ShieldCheck,
  Trash2,
  UserPlus,
  Users,
} from 'lucide-react'
import { useEffect, useMemo, useState, type FormEvent } from 'react'
import { toast } from 'sonner'
import { AppearancePanel } from './AppearancePanel'
import { relativeTime } from '@/lib/format'
import { desktopApi } from '@/lib/ipc'
import type { AlertEvent, MemberWithPassword, PairingCode, Role, ServerView } from '@/lib/types'

interface SettingsViewProps {
  server: ServerView
  onRemoveServer: (id: string) => void
}

type SettingsTab = 'general' | 'appearance' | 'team' | 'devices' | 'alerts' | 'audit'

const ALL_ALERT_EVENTS: { id: AlertEvent; label: string }[] = [
  { id: 'deploy_failed', label: 'Deploy failed' },
  { id: 'app_crashed', label: 'App crashed' },
  { id: 'app_down', label: 'App down' },
  { id: 'disk_low', label: 'Disk low' },
  { id: 'backup_failed', label: 'Backup failed' },
]

export function SettingsView({ server, onRemoveServer }: SettingsViewProps) {
  const [tab, setTab] = useState<SettingsTab>('general')
  const [pairing, setPairing] = useState<PairingCode | null>(null)
  const [creatingPair, setCreatingPair] = useState(false)

  // General org settings state
  const [orgName, setOrgName] = useState('')
  const [accentColor, setAccentColor] = useState('#10b981')
  const [envLabel, setEnvLabel] = useState('')
  const [savingOrg, setSavingOrg] = useState(false)

  // Team state
  const [memberName, setMemberName] = useState('')
  const [memberEmail, setMemberEmail] = useState('')
  const [memberRole, setMemberRole] = useState<Role>('developer')
  const [addingMember, setAddingMember] = useState(false)
  const [tempCreds, setTempCreds] = useState<MemberWithPassword | null>(null)

  // Alerts state
  const [alertName, setAlertName] = useState('')
  const [alertUrl, setAlertUrl] = useState('')
  const [alertEvents, setAlertEvents] = useState<AlertEvent[]>(['deploy_failed', 'app_crashed'])
  const [addingAlert, setAddingAlert] = useState(false)
  const [lastSigningSecret, setLastSigningSecret] = useState<string | null>(null)

  // Audit filter
  const [auditFilter, setAuditFilter] = useState('')

  const settingsQuery = useQuery({
    queryKey: ['desktop', server.id, 'settings'],
    queryFn: () => desktopApi.getServerSettings(server.id),
  })

  const data = settingsQuery.data

  useEffect(() => {
    if (!data?.me?.org) return
    setOrgName(data.me.org.name)
    setAccentColor(data.me.org.accent_color || '#10b981')
    setEnvLabel(data.me.org.env_label ?? '')
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

  async function saveOrg(e: FormEvent) {
    e.preventDefault()
    setSavingOrg(true)
    try {
      await desktopApi.updateOrganization(server.id, {
        name: orgName.trim() || undefined,
        accent_color: accentColor || undefined,
        env_label: envLabel.trim(),
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

  const tabs: { id: SettingsTab; label: string; icon: React.ReactNode }[] = [
    { id: 'general', label: 'General & Network', icon: <Globe className="size-3.5" /> },
    { id: 'appearance', label: 'Appearance', icon: <Palette className="size-3.5" /> },
    { id: 'team', label: `Team (${data?.members.length ?? 0})`, icon: <Users className="size-3.5" /> },
    {
      id: 'devices',
      label: `Devices & Tokens (${(data?.devices.length ?? 0) + (data?.tokens.length ?? 0)})`,
      icon: <Laptop className="size-3.5" />,
    },
    { id: 'alerts', label: `Alerts (${data?.alerts.length ?? 0})`, icon: <Bell className="size-3.5" /> },
    { id: 'audit', label: 'Audit Log', icon: <History className="size-3.5" /> },
  ]

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h1 className="text-lg font-semibold tracking-tight text-fg">Server Settings</h1>
          <p className="text-xs text-muted">
            Manage organization branding, network access, team members, paired devices, webhooks,
            and audit logs on <span className="font-medium text-fg">{server.name}</span>.
          </p>
        </div>
        <button
          type="button"
          disabled={settingsQuery.isFetching}
          onClick={() => void settingsQuery.refetch()}
          className="inline-flex items-center gap-1.5 rounded-lg border border-line bg-surface px-3 py-1.5 text-xs text-fg hover:bg-surface-2 disabled:opacity-50"
        >
          <RefreshCw className={`size-3.5 ${settingsQuery.isFetching ? 'animate-spin' : ''}`} />
          Refresh
        </button>
      </div>

      {/* Sub-navigation Tabs */}
      <div className="flex flex-wrap gap-1 border-b border-line pb-2">
        {tabs.map((t) => (
          <button
            key={t.id}
            type="button"
            onClick={() => setTab(t.id)}
            className={`inline-flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-xs font-medium transition-colors ${
              tab === t.id
                ? 'bg-surface-2 text-fg ring-1 ring-line'
                : 'text-muted hover:bg-surface hover:text-fg'
            }`}
          >
            {t.icon}
            {t.label}
          </button>
        ))}
      </div>

      {/* GENERAL & NETWORK */}
      {tab === 'general' && (
        <div className="space-y-4">
          <form
            onSubmit={saveOrg}
            className="space-y-4 rounded-xl border border-line bg-surface p-4 text-xs"
          >
            <div className="text-sm font-semibold text-fg">Organization & Branding</div>
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
              <div>
                <label className="mb-1 block text-muted">Organization Name</label>
                <input
                  type="text"
                  value={orgName}
                  onChange={(e) => setOrgName(e.target.value)}
                  placeholder="My Homelab"
                  className="w-full rounded-lg border border-line bg-bg px-3 py-1.5 text-xs text-fg outline-none focus:border-emerald-500"
                />
              </div>
              <div>
                <label className="mb-1 block text-muted">Environment Badge</label>
                <input
                  type="text"
                  value={envLabel}
                  onChange={(e) => setEnvLabel(e.target.value)}
                  placeholder="production, staging, homelab…"
                  className="w-full rounded-lg border border-line bg-bg px-3 py-1.5 text-xs text-fg outline-none focus:border-emerald-500"
                />
              </div>
              <div>
                <label className="mb-1 block text-muted">Accent Color</label>
                <div className="flex items-center gap-2">
                  <input
                    type="color"
                    value={accentColor}
                    onChange={(e) => setAccentColor(e.target.value)}
                    className="h-8 w-10 cursor-pointer rounded border border-line bg-bg"
                  />
                  <input
                    type="text"
                    value={accentColor}
                    onChange={(e) => setAccentColor(e.target.value)}
                    className="flex-1 rounded-lg border border-line bg-bg px-2.5 py-1.5 font-mono text-xs text-fg outline-none focus:border-emerald-500"
                  />
                </div>
              </div>
            </div>
            <div className="flex items-center justify-between border-t border-line pt-3">
              <span className="text-[11px] text-muted">
                Signed in as{' '}
                <span className="font-mono text-fg">
                  {data?.me?.user.email ?? server.user_email ?? '—'}
                </span>{' '}
                ({data?.me?.role ?? 'member'})
              </span>
              <button
                type="submit"
                disabled={savingOrg}
                className="inline-flex items-center gap-1.5 rounded-lg bg-emerald-600 px-3.5 py-1.5 font-medium text-white hover:bg-emerald-500 disabled:opacity-50"
              >
                <Save className="size-3.5" />
                {savingOrg ? 'Saving…' : 'Save Organization'}
              </button>
            </div>
          </form>

          {/* Network & TLS */}
          <div className="rounded-xl border border-line bg-surface p-4 text-xs">
            <div className="mb-3 flex items-center gap-2 font-semibold text-fg">
              <ShieldCheck className="size-4 text-emerald-400" />
              Network, Connection & Capabilities
            </div>
            <dl className="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <div>
                <dt className="text-muted">Connected Endpoint</dt>
                <dd className="mt-0.5 font-mono text-fg">{server.url}</dd>
              </div>
              <div>
                <dt className="text-muted">Server Version / Mode</dt>
                <dd className="mt-0.5 font-mono text-fg">
                  {server.meta
                    ? `v${server.meta.server_version} (API v${server.meta.api_version}) · ${server.meta.mode ?? 'full'} · ${server.meta.access ?? data?.network?.dashboard_access ?? '—'}`
                    : 'Unknown'}
                </dd>
              </div>
              {data?.network && (
                <>
                  <div>
                    <dt className="text-muted">LAN Addresses</dt>
                    <dd className="mt-0.5 font-mono text-fg">
                      {data.network.lan_addresses.length > 0
                        ? data.network.lan_addresses.join(', ')
                        : '—'}
                    </dd>
                  </div>
                  <div>
                    <dt className="text-muted">Tailscale</dt>
                    <dd className="mt-0.5 font-mono text-fg">
                      {data.network.tailscale?.dns_name ||
                        data.network.tailscale?.addresses.join(', ') ||
                        'Not detected'}
                    </dd>
                  </div>
                </>
              )}
              <div>
                <dt className="text-muted">TLS Pinning</dt>
                <dd className="mt-0.5 break-all font-mono text-[11px] text-fg">
                  {server.pinned_tls_sha256 ?? 'Standard Public WebPKI Roots'}
                </dd>
              </div>
              <div>
                <dt className="text-muted">Capabilities</dt>
                <dd className="mt-1 flex flex-wrap gap-1">
                  {(server.meta?.capabilities ?? []).map((c) => (
                    <span
                      key={c}
                      className="rounded bg-surface-2 px-1.5 py-0.5 font-mono text-[10px] text-muted"
                    >
                      {c}
                    </span>
                  ))}
                </dd>
              </div>
            </dl>
          </div>

          {/* Disconnect Danger Zone */}
          <div className="flex items-center justify-between rounded-xl border border-rose-500/25 bg-rose-500/5 px-4 py-3 text-xs">
            <div>
              <div className="font-semibold text-rose-300">Disconnect from Rig Desktop</div>
              <div className="text-muted">
                Removes this server from the switcher and deletes its token from your OS keychain.
              </div>
            </div>
            <button
              type="button"
              onClick={() => onRemoveServer(server.id)}
              className="rounded-lg bg-rose-600 px-3 py-1.5 font-medium text-white hover:bg-rose-500"
            >
              Remove Server
            </button>
          </div>
        </div>
      )}

      {/* TEAM & MEMBERS */}
      {tab === 'team' && (
        <div className="space-y-4">
          <form
            onSubmit={handleAddMember}
            className="flex flex-wrap items-center gap-2 rounded-xl border border-line bg-surface p-3.5 text-xs"
          >
            <input
              type="text"
              required
              value={memberName}
              onChange={(e) => setMemberName(e.target.value)}
              placeholder="Full name"
              className="w-44 rounded-lg border border-line bg-bg px-3 py-1.5 text-xs text-fg outline-none focus:border-emerald-500"
            />
            <input
              type="email"
              required
              value={memberEmail}
              onChange={(e) => setMemberEmail(e.target.value)}
              placeholder="member@company.com"
              className="min-w-[200px] flex-1 rounded-lg border border-line bg-bg px-3 py-1.5 text-xs text-fg outline-none focus:border-emerald-500"
            />
            <select
              value={memberRole}
              onChange={(e) => setMemberRole(e.target.value as Role)}
              className="rounded-lg border border-line bg-bg px-2.5 py-1.5 text-xs text-fg"
            >
              <option value="viewer">Viewer</option>
              <option value="developer">Developer</option>
              <option value="admin">Admin</option>
              <option value="owner">Owner</option>
            </select>
            <button
              type="submit"
              disabled={addingMember}
              className="inline-flex items-center gap-1.5 rounded-lg bg-emerald-600 px-3 py-1.5 font-medium text-white hover:bg-emerald-500 disabled:opacity-50"
            >
              <UserPlus className="size-3.5" />
              Add Member
            </button>
          </form>

          {tempCreds && (
            <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-emerald-500/30 bg-emerald-500/10 px-4 py-3 text-xs">
              <div>
                <div className="font-semibold text-emerald-200">
                  Temporary password for {tempCreds.member.email}
                </div>
                <div className="mt-0.5 font-mono text-sm text-emerald-100">
                  {tempCreds.temporary_password}
                </div>
              </div>
              <button
                type="button"
                onClick={() => {
                  void navigator.clipboard.writeText(tempCreds.temporary_password)
                  toast.success('Copied temporary password')
                }}
                className="inline-flex items-center gap-1.5 rounded-lg border border-emerald-400/30 bg-bg/60 px-3 py-1.5 text-emerald-200 hover:bg-bg"
              >
                <Copy className="size-3.5" />
                Copy Password
              </button>
            </div>
          )}

          <div className="divide-y divide-line overflow-hidden rounded-xl border border-line bg-surface text-xs">
            {(data?.members ?? []).length === 0 ? (
              <div className="px-4 py-8 text-center text-muted">
                No members returned (requires admin/owner role).
              </div>
            ) : (
              (data?.members ?? []).map((m) => (
                <div
                  key={m.user_id}
                  className="flex flex-wrap items-center justify-between gap-3 px-4 py-3"
                >
                  <div>
                    <div className="font-medium text-fg">{m.name}</div>
                    <div className="font-mono text-[11px] text-muted">
                      {m.email} · last login {relativeTime(m.last_login_at)}
                    </div>
                  </div>
                  <div className="flex items-center gap-2">
                    <select
                      value={m.role}
                      onChange={async (e) => {
                        const role = e.target.value as Role
                        try {
                          await desktopApi.updateMember(server.id, m.user_id, { role })
                          toast.success(`Updated ${m.name} to ${role}`)
                          void settingsQuery.refetch()
                        } catch (err) {
                          toast.error(String(err))
                        }
                      }}
                      className="rounded-md border border-line bg-bg px-2 py-1 text-[11px] text-fg"
                    >
                      <option value="viewer">viewer</option>
                      <option value="developer">developer</option>
                      <option value="admin">admin</option>
                      <option value="owner">owner</option>
                    </select>
                    <button
                      type="button"
                      onClick={async () => {
                        try {
                          const res = await desktopApi.resetMemberPassword(server.id, m.user_id)
                          setTempCreds(res)
                          toast.success(`Reset password for ${m.name}`)
                        } catch (err) {
                          toast.error(String(err))
                        }
                      }}
                      className="rounded-md border border-line bg-surface-2 px-2 py-1 text-[11px] text-fg hover:bg-bg"
                    >
                      Reset Password
                    </button>
                    <button
                      type="button"
                      onClick={async () => {
                        try {
                          await desktopApi.removeMember(server.id, m.user_id)
                          toast.success(`Removed ${m.name}`)
                          void settingsQuery.refetch()
                        } catch (err) {
                          toast.error(String(err))
                        }
                      }}
                      className="rounded-md p-1.5 text-muted hover:bg-rose-500/15 hover:text-rose-300"
                    >
                      <Trash2 className="size-3.5" />
                    </button>
                  </div>
                </div>
              ))
            )}
          </div>
        </div>
      )}

      {/* DEVICES & API TOKENS */}
      {tab === 'devices' && (
        <div className="space-y-4">
          <div className="rounded-xl border border-line bg-surface">
            <div className="flex flex-wrap items-center justify-between gap-2 border-b border-line px-4 py-3">
              <div className="flex items-center gap-2 text-xs font-semibold text-fg">
                <Laptop className="size-4 text-emerald-400" />
                Paired Devices ({data?.devices.length ?? 0})
              </div>
              <button
                type="button"
                disabled={creatingPair}
                onClick={generateCode}
                className="inline-flex items-center gap-1.5 rounded-lg bg-emerald-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-emerald-500 disabled:opacity-50"
              >
                <KeyRound className="size-3.5" />
                {creatingPair ? 'Generating…' : 'Pair Another Device'}
              </button>
            </div>

            {pairing && (
              <div className="flex flex-wrap items-center justify-between gap-3 border-b border-emerald-500/25 bg-emerald-500/10 px-4 py-3 text-xs">
                <div>
                  <div className="text-[11px] text-emerald-200/80">
                    One-time pairing code (single use, valid 5 min)
                  </div>
                  <div className="mt-0.5 font-mono text-lg font-bold tracking-[0.2em] text-emerald-200">
                    {pairing.code}
                  </div>
                </div>
                <button
                  type="button"
                  onClick={() => {
                    void navigator.clipboard.writeText(pairing.code)
                    toast.success('Copied pairing code')
                  }}
                  className="inline-flex items-center gap-1.5 rounded-lg border border-emerald-400/30 bg-bg/60 px-3 py-1.5 text-xs text-emerald-200 hover:bg-bg"
                >
                  <Copy className="size-3.5" />
                  Copy Code
                </button>
              </div>
            )}

            <div className="divide-y divide-line text-xs">
              {(data?.devices ?? []).length === 0 ? (
                <div className="px-4 py-6 text-center text-muted">No paired devices found.</div>
              ) : (
                (data?.devices ?? []).map((dev) => (
                  <div key={dev.id} className="flex items-center justify-between gap-3 px-4 py-2.5">
                    <div className="min-w-0">
                      <div className="font-medium text-fg">{dev.name}</div>
                      <div className="font-mono text-[11px] text-muted">
                        {dev.prefix}… · {dev.role} · IP {dev.last_ip ?? '—'} · last seen{' '}
                        {relativeTime(dev.last_used_at)}
                      </div>
                    </div>
                    <button
                      type="button"
                      onClick={async () => {
                        try {
                          await desktopApi.revokeDevice(server.id, dev.id)
                          toast.success(`Revoked ${dev.name}`)
                          void settingsQuery.refetch()
                        } catch (e) {
                          toast.error(String(e))
                        }
                      }}
                      className="rounded-md p-1.5 text-muted hover:bg-rose-500/15 hover:text-rose-300"
                      title="Revoke device token"
                    >
                      <Trash2 className="size-3.5" />
                    </button>
                  </div>
                ))
              )}
            </div>
          </div>

          <div className="rounded-xl border border-line bg-surface">
            <div className="flex items-center gap-2 border-b border-line px-4 py-3 text-xs font-semibold text-fg">
              <KeyRound className="size-4 text-sky-400" />
              API & CLI Tokens ({data?.tokens.length ?? 0})
            </div>
            <div className="divide-y divide-line text-xs">
              {(data?.tokens ?? []).length === 0 ? (
                <div className="px-4 py-6 text-center text-muted">No API tokens found.</div>
              ) : (
                (data?.tokens ?? []).map((tok) => (
                  <div key={tok.id} className="flex items-center justify-between gap-3 px-4 py-2.5">
                    <div className="min-w-0">
                      <div className="flex items-center gap-2 font-medium text-fg">
                        <span>{tok.name}</span>
                        {tok.kind && (
                          <span className="rounded bg-surface-2 px-1.5 py-0.5 font-mono text-[10px] text-muted">
                            {tok.kind}
                          </span>
                        )}
                      </div>
                      <div className="font-mono text-[11px] text-muted">
                        {tok.prefix}… · {tok.role} · created {relativeTime(tok.created_at)}
                      </div>
                    </div>
                    <button
                      type="button"
                      onClick={async () => {
                        try {
                          await desktopApi.revokeToken(server.id, tok.id)
                          toast.success(`Revoked token ${tok.name}`)
                          void settingsQuery.refetch()
                        } catch (e) {
                          toast.error(String(e))
                        }
                      }}
                      className="rounded-md p-1.5 text-muted hover:bg-rose-500/15 hover:text-rose-300"
                      title="Revoke token"
                    >
                      <Trash2 className="size-3.5" />
                    </button>
                  </div>
                ))
              )}
            </div>
          </div>
        </div>
      )}

      {/* ALERTS */}
      {tab === 'alerts' && (
        <div className="space-y-4">
          <form
            onSubmit={handleAddAlert}
            className="space-y-3 rounded-xl border border-line bg-surface p-4 text-xs"
          >
            <div className="font-semibold text-fg">Add Webhook Alert Channel</div>
            <div className="grid grid-cols-1 gap-2 sm:grid-cols-3">
              <input
                type="text"
                required
                value={alertName}
                onChange={(e) => setAlertName(e.target.value)}
                placeholder="Channel name (e.g. Discord Ops)"
                className="rounded-lg border border-line bg-bg px-3 py-1.5 text-xs text-fg outline-none focus:border-emerald-500"
              />
              <input
                type="url"
                required
                value={alertUrl}
                onChange={(e) => setAlertUrl(e.target.value)}
                placeholder="https://discord.com/api/webhooks/..."
                className="rounded-lg border border-line bg-bg px-3 py-1.5 font-mono text-xs text-fg outline-none focus:border-emerald-500 sm:col-span-2"
              />
            </div>
            <div className="flex flex-wrap items-center justify-between gap-2">
              <div className="flex flex-wrap items-center gap-3">
                {ALL_ALERT_EVENTS.map((ev) => {
                  const checked = alertEvents.includes(ev.id)
                  return (
                    <label
                      key={ev.id}
                      className="inline-flex cursor-pointer items-center gap-1.5 text-muted hover:text-fg"
                    >
                      <input
                        type="checkbox"
                        checked={checked}
                        onChange={(e) => {
                          setAlertEvents((prev) =>
                            e.target.checked ? [...prev, ev.id] : prev.filter((x) => x !== ev.id),
                          )
                        }}
                        className="accent-emerald-500"
                      />
                      {ev.label}
                    </label>
                  )
                })}
              </div>
              <button
                type="submit"
                disabled={addingAlert || alertEvents.length === 0}
                className="inline-flex items-center gap-1.5 rounded-lg bg-emerald-600 px-3 py-1.5 font-medium text-white hover:bg-emerald-500 disabled:opacity-50"
              >
                <Plus className="size-3.5" />
                Add Channel
              </button>
            </div>
          </form>

          {lastSigningSecret && (
            <div className="flex items-center justify-between gap-2 rounded-xl border border-emerald-500/30 bg-emerald-500/10 px-4 py-2.5 text-xs">
              <div className="font-mono text-emerald-200">
                Webhook signing secret: <span className="font-bold">{lastSigningSecret}</span>
              </div>
              <button
                type="button"
                onClick={() => {
                  void navigator.clipboard.writeText(lastSigningSecret)
                  toast.success('Copied signing secret')
                }}
                className="rounded border border-emerald-400/30 px-2 py-1 text-emerald-200 hover:bg-bg/50"
              >
                Copy
              </button>
            </div>
          )}

          <div className="divide-y divide-line overflow-hidden rounded-xl border border-line bg-surface text-xs">
            {(data?.alerts ?? []).length === 0 ? (
              <div className="px-4 py-8 text-center text-muted">
                No alert channels configured yet.
              </div>
            ) : (
              (data?.alerts ?? []).map((ch) => (
                <div
                  key={ch.id}
                  className="flex flex-wrap items-center justify-between gap-3 px-4 py-3"
                >
                  <div>
                    <div className="flex items-center gap-2 font-medium text-fg">
                      <span>{ch.name}</span>
                      <span className="font-mono text-[11px] text-muted">{ch.url_host}</span>
                    </div>
                    <div className="mt-1 flex flex-wrap gap-1">
                      {ch.events.map((ev) => (
                        <span
                          key={ev}
                          className="rounded bg-surface-2 px-1.5 py-0.5 font-mono text-[10px] text-muted"
                        >
                          {ev}
                        </span>
                      ))}
                    </div>
                    {ch.last_error && (
                      <div className="mt-1 text-[11px] text-rose-300">{ch.last_error}</div>
                    )}
                  </div>
                  <div className="flex items-center gap-1.5">
                    <button
                      type="button"
                      onClick={async () => {
                        try {
                          const res = await desktopApi.testAlertChannel(server.id, ch.id)
                          if (res.ok) toast.success(`Test sent to ${ch.name}`)
                          else toast.error(res.error ?? 'Test failed')
                        } catch (err) {
                          toast.error(String(err))
                        }
                      }}
                      className="inline-flex items-center gap-1 rounded-md border border-line bg-surface-2 px-2.5 py-1 text-[11px] text-fg hover:bg-bg"
                    >
                      <Send className="size-3" />
                      Test
                    </button>
                    <button
                      type="button"
                      onClick={async () => {
                        try {
                          await desktopApi.deleteAlertChannel(server.id, ch.id)
                          toast.success(`Deleted ${ch.name}`)
                          void settingsQuery.refetch()
                        } catch (err) {
                          toast.error(String(err))
                        }
                      }}
                      className="rounded-md p-1.5 text-muted hover:bg-rose-500/15 hover:text-rose-300"
                    >
                      <Trash2 className="size-3.5" />
                    </button>
                  </div>
                </div>
              ))
            )}
          </div>
        </div>
      )}

      {/* AUDIT LOG */}
      {tab === 'audit' && (
        <div className="space-y-3">
          <input
            type="text"
            value={auditFilter}
            onChange={(e) => setAuditFilter(e.target.value)}
            placeholder="Filter audit log by actor, action, target, or detail…"
            className="w-full rounded-xl border border-line bg-surface px-3.5 py-2 text-xs text-fg outline-none focus:border-emerald-500"
          />
          <div className="divide-y divide-line overflow-hidden rounded-xl border border-line bg-surface text-xs">
            {filteredAudit.length === 0 ? (
              <div className="px-4 py-8 text-center text-muted">No audit entries found.</div>
            ) : (
              filteredAudit.map((entry) => (
                <div
                  key={entry.id}
                  className="flex flex-wrap items-center justify-between gap-2 px-4 py-2.5"
                >
                  <div className="min-w-0">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="font-mono font-semibold text-emerald-300">
                        {entry.action}
                      </span>
                      <span className="text-fg">{entry.actor}</span>
                      {entry.target && (
                        <span className="font-mono text-[11px] text-muted">{entry.target}</span>
                      )}
                    </div>
                    {entry.detail && (
                      <div className="mt-0.5 truncate text-[11px] text-muted">{entry.detail}</div>
                    )}
                  </div>
                  <div className="shrink-0 font-mono text-[10px] text-muted">
                    {entry.ip ? `${entry.ip} · ` : ''}
                    {relativeTime(entry.created_at)}
                  </div>
                </div>
              ))
            )}
          </div>
        </div>
      )}

      {/* APPEARANCE & UI CUSTOMIZATION */}
      {tab === 'appearance' && <AppearancePanel />}
    </div>
  )
}
