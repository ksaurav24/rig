import { useQuery, useQueryClient } from '@tanstack/react-query'
import {
  ArrowDown,
  ArrowUp,
  Bell,
  Check,
  Clock,
  Copy,
  Download,
  ExternalLink,
  Home,
  Info,
  Layers,
  Minus,
  Monitor,
  Moon,
  Palette,
  PanelLeft,
  Pencil,
  Plus,
  RefreshCw,
  RotateCcw,
  Server,
  ShieldCheck,
  Sliders,
  Square,
  Sun,
  Terminal,
  TerminalSquare,
  Trash2,
  X,
  ZoomIn,
} from 'lucide-react'
import { useState, type ComponentType } from 'react'
import { toast } from 'sonner'
import { Page, Panel } from '@rig/ui/app/page'
import { EmptyState, Status, statusTone } from '@rig/ui/app/state'
import { Badge } from '@rig/ui/badge'
import { Banner, BannerDescription, BannerTitle } from '@rig/ui/banner'
import { Button } from '@rig/ui/button'
import { ConfirmDialog } from '@rig/ui/confirm-dialog'
import { Input } from '@rig/ui/input'
import { Switch } from '@rig/ui/switch'
import { Tabs, TabsItem } from '@rig/ui/tabs'
import { ACCENT_COLORS, accentHex, formatDurationMs, formatExactTime, initials, relativeTime } from '@/lib/format'
import { desktopApi } from '@/lib/ipc'
import {
  ACCENT_PRESETS,
  MAX_ZOOM,
  MIN_ZOOM,
  SURFACE_PRESETS,
  ZOOM_PRESETS,
  usePreferences,
  type DefaultSection,
  type LogFontSize,
  type SidebarLayout,
  type ThemeMode,
  type TimeStyle,
  type TimeZoneMode,
  type UiDensity,
} from '@/lib/preferences'
import type { ShapeVariant } from '@rig/ui/lib/shape-context'
import type { ServerView, UpdateCheckResult } from '@/lib/types'
import { cn } from '@rig/ui/lib/utils'

export type DesktopSettingsTab = 'general' | 'appearance' | 'servers' | 'cli' | 'updates'

interface DesktopSettingsViewProps {
  initialTab?: DesktopSettingsTab
  servers: ServerView[]
  activeServerId: string | null
  onSelectServer: (id: string) => void
  onAddServer: () => void
  onRemoveServer: (id: string) => void
  onReorderServers: (orderedIds: string[]) => void
}

const SERVER_ACCENT_KEYS = Object.keys(ACCENT_COLORS)

function SegmentedTabs<T extends string | number>({
  value,
  options,
  onChange,
}: {
  value: T
  options: {
    value: T
    label: string
    icon?: ComponentType<{ size?: number; strokeWidth?: number; className?: string }>
  }[]
  onChange: (next: T) => void
}) {
  const selectedIndex = Math.max(
    0,
    options.findIndex((o) => o.value === value),
  )
  return (
    <div className="inline-flex rounded-lg border border-border/80 bg-surface p-0.5">
      <Tabs
        size="compact"
        className="mx-0 my-0 max-w-full px-0 py-0"
        selectedIndex={selectedIndex}
        onSelect={(idx) => {
          const opt = options[idx]
          if (opt) onChange(opt.value)
        }}
      >
        {options.map((opt, idx) => (
          <TabsItem key={String(opt.value)} index={idx} label={opt.label} icon={opt.icon} />
        ))}
      </Tabs>
    </div>
  )
}

function SettingRow({
  icon: Icon,
  title,
  description,
  children,
  stacked = false,
}: {
  icon: ComponentType<{ className?: string; strokeWidth?: number }>
  title: string
  description: React.ReactNode
  children: React.ReactNode
  stacked?: boolean
}) {
  return (
    <div
      className={cn(
        'px-5 py-4',
        stacked ? 'space-y-3' : 'flex flex-wrap items-center justify-between gap-4',
      )}
    >
      <div className="flex items-start gap-3">
        <div className="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-lg bg-hover text-primary">
          <Icon className="size-4" strokeWidth={1.75} />
        </div>
        <div className="min-w-0">
          <div className="text-[14px] font-medium text-foreground">{title}</div>
          <div className="mt-0.5 text-[12.5px] text-muted-foreground">{description}</div>
        </div>
      </div>
      <div className={cn(stacked ? 'pl-11' : 'shrink-0')}>{children}</div>
    </div>
  )
}

function CopyableCodeRow({ label, command }: { label: string; command: string }) {
  return (
    <div className="flex items-center justify-between gap-3 rounded-lg border border-border/80 bg-surface/70 px-3.5 py-2.5">
      <div className="min-w-0">
        <div className="text-[11.5px] font-medium text-muted-foreground">{label}</div>
        <code className="mt-0.5 block truncate font-mono text-[12.5px] text-foreground">
          {command}
        </code>
      </div>
      <Button
        type="button"
        variant="ghost"
        size="icon-compact"
        aria-label={`Copy ${label}`}
        onClick={() => {
          void navigator.clipboard.writeText(command)
          toast.success(`Copied ${label}`)
        }}
      >
        <Copy />
      </Button>
    </div>
  )
}

export function DesktopSettingsView({
  initialTab = 'general',
  servers,
  activeServerId,
  onSelectServer,
  onAddServer,
  onRemoveServer,
  onReorderServers,
}: DesktopSettingsViewProps) {
  const qc = useQueryClient()
  const { prefs, resolvedTheme, updatePrefs, zoomIn, zoomOut, resetZoom, resetPrefs } =
    usePreferences()
  const [tab, setTab] = useState<DesktopSettingsTab>(initialTab)
  const [editingServerId, setEditingServerId] = useState<string | null>(null)
  const [editName, setEditName] = useState('')
  const [editAccent, setEditAccent] = useState('emerald')
  const [savingServer, setSavingServer] = useState(false)
  const [confirmRemove, setConfirmRemove] = useState<ServerView | null>(null)
  const [updateResult, setUpdateResult] = useState<UpdateCheckResult | null>(null)
  const [checkingUpdates, setCheckingUpdates] = useState(false)

  const appInfoQuery = useQuery({
    queryKey: ['desktop', 'app-info'],
    queryFn: () => desktopApi.getAppInfo(),
  })

  const perfQuery = useQuery({
    queryKey: ['desktop', 'perf-diagnostics'],
    queryFn: () => desktopApi.getPerfDiagnostics(),
    enabled: tab === 'updates',
  })

  const sampleIso = new Date(Date.now() - 4 * 60_000).toISOString()
  const zoomPercent = Math.round(prefs.zoom * 100)
  const appInfo = appInfoQuery.data

  const tabs: {
    id: DesktopSettingsTab
    label: string
    icon: ComponentType<{ size?: number; strokeWidth?: number; className?: string }>
  }[] = [
    { id: 'general', label: 'General', icon: Sliders },
    { id: 'appearance', label: 'Appearance', icon: Palette },
    { id: 'servers', label: `Servers (${servers.length})`, icon: Server },
    { id: 'cli', label: 'CLI & MCP', icon: Terminal },
    { id: 'updates', label: 'Versions & Updates', icon: Info },
  ]
  const tabIndex = Math.max(
    0,
    tabs.findIndex((t) => t.id === tab),
  )

  function beginEditServer(s: ServerView) {
    setEditingServerId(s.id)
    setEditName(s.name)
    setEditAccent(s.accent || 'emerald')
  }

  async function saveEditServer(id: string) {
    if (!editName.trim()) return
    setSavingServer(true)
    try {
      await desktopApi.updateServer(id, {
        name: editName.trim(),
        accent: editAccent,
      })
      toast.success('Updated server profile')
      setEditingServerId(null)
      await qc.invalidateQueries({ queryKey: ['desktop', 'servers'] })
    } catch (e) {
      toast.error(String(e))
    } finally {
      setSavingServer(false)
    }
  }

  function moveServer(index: number, delta: -1 | 1) {
    const target = index + delta
    if (target < 0 || target >= servers.length) return
    const ids = servers.map((s) => s.id)
    const [moved] = ids.splice(index, 1)
    ids.splice(target, 0, moved)
    onReorderServers(ids)
  }

  async function runUpdateCheck() {
    setCheckingUpdates(true)
    try {
      const res = await desktopApi.checkDesktopUpdates()
      setUpdateResult(res)
      if (res.error) {
        toast.error(res.error)
      } else if (res.update_available && res.latest_version) {
        toast.info(`Rig v${res.latest_version} is available`)
      } else {
        toast.success(`Rig Desktop v${res.current_version} is up to date`)
      }
    } catch (e) {
      toast.error(String(e))
    } finally {
      setCheckingUpdates(false)
    }
  }

  const activeServer = servers.find((s) => s.id === activeServerId) ?? servers[0] ?? null
  const mcpSnippet = JSON.stringify(
    {
      mcpServers: {
        rig: {
          command: appInfo?.cli.binary_path || 'rig',
          args: ['mcp'],
        },
      },
    },
    null,
    2,
  )

  return (
    <Page
      title="App Settings"
      description="Configure Rig Desktop preferences, appearance, paired servers, local CLI integration, and version diagnostics. Saved locally on this machine."
      actions={
        appInfo && (
          <Badge variant="solid" color="gray" size="compact" className="font-mono">
            v{appInfo.app_version} · {appInfo.os}/{appInfo.arch}
          </Badge>
        )
      }
    >
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

      {/* GENERAL */}
      {tab === 'general' && (
        <div className="space-y-5">
          <Panel title="Workspace & Behavior" flush>
            <div className="divide-y divide-border/80">
              <SettingRow
                icon={Home}
                title="Default Landing View"
                description="Which server screen opens first when launching Rig Desktop or adding a server."
              >
                <SegmentedTabs<DefaultSection>
                  value={prefs.defaultSection}
                  onChange={(defaultSection) => updatePrefs({ defaultSection })}
                  options={[
                    { value: 'overview', label: 'Overview' },
                    { value: 'apps', label: 'Apps' },
                    { value: 'local', label: 'Local Repos' },
                    { value: 'databases', label: 'Databases' },
                  ]}
                />
              </SettingRow>

              <SettingRow
                icon={PanelLeft}
                title="Navigation Sidebar"
                description="Choose expanded labels or an icon-only rail, and toggle keyboard shortcut hints."
              >
                <div className="flex flex-wrap items-center gap-3">
                  <SegmentedTabs<SidebarLayout>
                    value={prefs.sidebarLayout}
                    onChange={(sidebarLayout) => updatePrefs({ sidebarLayout })}
                    options={[
                      { value: 'expanded', label: 'Expanded' },
                      { value: 'compact', label: 'Icon only' },
                    ]}
                  />
                  <Switch
                    label="Shortcut hints"
                    size="compact"
                    checked={prefs.showShortcutHints}
                    onToggle={() => updatePrefs({ showShortcutHints: !prefs.showShortcutHints })}
                  />
                </div>
              </SettingRow>

              <SettingRow
                icon={Bell}
                title="Event Notifications"
                description="Show in-app alerts when deployments succeed, fail, or containers crash across paired servers."
              >
                <Switch
                  label={prefs.notificationsEnabled ? 'Enabled' : 'Muted'}
                  size="compact"
                  checked={prefs.notificationsEnabled}
                  onToggle={() =>
                    updatePrefs({ notificationsEnabled: !prefs.notificationsEnabled })
                  }
                />
              </SettingRow>
            </div>
          </Panel>

          <Panel title="Logs, Terminal & Time" flush>
            <div className="divide-y divide-border/80">
              <SettingRow
                icon={TerminalSquare}
                title="Logs & Terminal"
                description="Monospace font size and default line wrapping in virtualized logs and container shells."
              >
                <div className="flex flex-wrap items-center gap-3">
                  <SegmentedTabs<LogFontSize>
                    value={prefs.logFontSize}
                    onChange={(logFontSize) => updatePrefs({ logFontSize })}
                    options={[
                      { value: 12, label: '12px' },
                      { value: 13, label: '13px' },
                      { value: 14, label: '14px' },
                      { value: 15, label: '15px' },
                    ]}
                  />
                  <Switch
                    label="Wrap lines"
                    size="compact"
                    checked={prefs.logWrap}
                    onToggle={() => updatePrefs({ logWrap: !prefs.logWrap })}
                  />
                </div>
              </SettingRow>

              <SettingRow
                icon={Clock}
                title="Timestamps"
                description={
                  <>
                    Preview:{' '}
                    <span className="font-mono text-foreground">{relativeTime(sampleIso)}</span> ·{' '}
                    <span className="font-mono">{formatExactTime(sampleIso)}</span>
                  </>
                }
              >
                <div className="flex flex-wrap items-center gap-2">
                  <SegmentedTabs<TimeStyle>
                    value={prefs.timeStyle}
                    onChange={(timeStyle) => updatePrefs({ timeStyle })}
                    options={[
                      { value: 'relative', label: 'Relative' },
                      { value: 'exact', label: 'Exact' },
                    ]}
                  />
                  <SegmentedTabs<TimeZoneMode>
                    value={prefs.timeZone}
                    onChange={(timeZone) => updatePrefs({ timeZone })}
                    options={[
                      { value: 'local', label: 'Local' },
                      { value: 'utc', label: 'UTC' },
                    ]}
                  />
                </div>
              </SettingRow>
            </div>
          </Panel>

          <div className="flex justify-end">
            <Button
              type="button"
              variant="tertiary"
              leadingIcon={RotateCcw}
              onClick={() => {
                resetPrefs()
                toast.success('Reset desktop preferences to defaults')
              }}
            >
              Reset All App Defaults
            </Button>
          </div>
        </div>
      )}

      {/* APPEARANCE */}
      {tab === 'appearance' && (
        <div className="space-y-5">
          <Panel flush>
            <div className="divide-y divide-border/80">
              <SettingRow
                icon={ZoomIn}
                title="Interface Zoom"
                description="Scale the entire desktop window smoothly for your display resolution. Use Ctrl + / Ctrl − / Ctrl 0 or Ctrl + Scroll anytime."
                stacked
              >
                <div className="flex flex-wrap items-center gap-2.5">
                  <div className="inline-flex items-center gap-1 rounded-lg border border-border bg-surface p-1">
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon-compact"
                      onClick={zoomOut}
                      disabled={prefs.zoom <= MIN_ZOOM}
                      aria-label="Zoom out"
                    >
                      <Minus />
                    </Button>
                    <span className="min-w-[54px] text-center font-mono text-[13px] font-medium text-foreground tabular-nums">
                      {zoomPercent}%
                    </span>
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon-compact"
                      onClick={zoomIn}
                      disabled={prefs.zoom >= MAX_ZOOM}
                      aria-label="Zoom in"
                    >
                      <Plus />
                    </Button>
                  </div>

                  <div className="flex flex-wrap items-center gap-1.5">
                    {ZOOM_PRESETS.map((preset) => {
                      const active = Math.abs(prefs.zoom - preset) < 0.01
                      return (
                        <Button
                          key={preset}
                          type="button"
                          variant={active ? 'primary' : 'tertiary'}
                          size="compact"
                          onClick={() => updatePrefs({ zoom: preset })}
                          className="font-mono tabular-nums"
                        >
                          {Math.round(preset * 100)}%
                        </Button>
                      )
                    })}
                    {prefs.zoom !== 1 && (
                      <Button type="button" variant="ghost" size="compact" onClick={resetZoom}>
                        Reset
                      </Button>
                    )}
                  </div>
                </div>
              </SettingRow>

              <SettingRow
                icon={Sun}
                title="Theme"
                description="Switch between crisp dark, warm light, or follow your operating system."
              >
                <SegmentedTabs<ThemeMode>
                  value={prefs.theme}
                  onChange={(theme) => updatePrefs({ theme })}
                  options={[
                    { value: 'dark', label: 'Dark', icon: Moon },
                    { value: 'light', label: 'Light', icon: Sun },
                    { value: 'system', label: 'System', icon: Monitor },
                  ]}
                />
              </SettingRow>

              {resolvedTheme === 'dark' && (
                <SettingRow
                  icon={Layers}
                  title="Dark Surface Tone"
                  description="Warm Graphite matches the Rig web dashboard by default."
                  stacked
                >
                  <div className="grid grid-cols-2 gap-2.5 sm:grid-cols-4">
                    {SURFACE_PRESETS.map((preset) => {
                      const active = prefs.surface === preset.value
                      return (
                        <button
                          key={preset.value}
                          type="button"
                          onClick={() => updatePrefs({ surface: preset.value })}
                          className={cn(
                            'flex items-center gap-2.5 rounded-lg border px-3 py-2.5 text-left text-[13px] transition-colors duration-80',
                            active
                              ? 'border-primary bg-primary/10 font-medium text-foreground'
                              : 'border-border bg-surface text-muted-foreground hover:bg-hover hover:text-foreground',
                          )}
                        >
                          <span
                            className="flex size-5 shrink-0 items-end justify-end rounded border border-white/15 p-0.5"
                            style={{ backgroundColor: preset.bg }}
                          >
                            <span
                              className="size-2.5 rounded-[2px]"
                              style={{ backgroundColor: preset.card }}
                            />
                          </span>
                          <span className="truncate">{preset.label}</span>
                        </button>
                      )
                    })}
                  </div>
                </SettingRow>
              )}

              <SettingRow
                icon={Palette}
                title="Accent Color"
                description="Curated OKLCH accents applied across primary indicators, focus rings, and switches."
                stacked
              >
                <div className="flex flex-wrap gap-2">
                  {ACCENT_PRESETS.map((preset) => {
                    const active = prefs.accent === preset.value
                    const swatch =
                      resolvedTheme === 'dark' ? preset.swatchDark : preset.swatchLight
                    return (
                      <button
                        key={preset.value}
                        type="button"
                        onClick={() => updatePrefs({ accent: preset.value })}
                        className={cn(
                          'inline-flex items-center gap-2 rounded-full border px-3 py-1.5 text-[13px] font-medium transition-colors duration-80',
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
              </SettingRow>

              <SettingRow
                icon={Sliders}
                title="Control Density"
                description="Choose compact 28px controls, balanced 36px defaults, or comfortable larger type."
              >
                <SegmentedTabs<UiDensity>
                  value={prefs.density}
                  onChange={(density) => updatePrefs({ density })}
                  options={[
                    { value: 'compact', label: 'Compact' },
                    { value: 'default', label: 'Default' },
                    { value: 'comfortable', label: 'Comfortable' },
                  ]}
                />
              </SettingRow>

              <SettingRow
                icon={Square}
                title="Corner Shape"
                description="Live Fluid Functionalism corner geometry across buttons, inputs, tabs, and menus."
              >
                <SegmentedTabs<ShapeVariant>
                  value={prefs.shape}
                  onChange={(shape) => updatePrefs({ shape })}
                  options={[
                    { value: 'sharp', label: 'Sharp' },
                    { value: 'rounded', label: 'Rounded' },
                    { value: 'pill', label: 'Pill' },
                  ]}
                />
              </SettingRow>
            </div>
          </Panel>
        </div>
      )}

      {/* SERVERS */}
      {tab === 'servers' && (
        <div className="space-y-5">
          <Panel
            title="Paired Rig Servers"
            description="Reorder, rename, customize rail accents, or remove servers paired with this device."
            actions={
              <Button
                type="button"
                variant="primary"
                size="compact"
                leadingIcon={Plus}
                onClick={onAddServer}
              >
                Add Server
              </Button>
            }
            flush={servers.length > 0}
          >
            {servers.length === 0 ? (
              <EmptyState
                icon={Server}
                title="No Rig servers paired yet"
                action={
                  <Button type="button" variant="primary" leadingIcon={Plus} onClick={onAddServer}>
                    Add Rig Server
                  </Button>
                }
              >
                Pair a headless or full Rig server over public HTTPS, LAN internal CA, or Tailscale.
              </EmptyState>
            ) : (
              <div className="divide-y divide-border/80">
                {servers.map((srv, idx) => {
                  const isEditing = editingServerId === srv.id
                  const isActive = srv.id === activeServerId
                  const color = accentHex(srv.accent)

                  if (isEditing) {
                    return (
                      <div key={srv.id} className="space-y-3 bg-hover/60 px-5 py-4">
                        <div className="flex flex-wrap items-center gap-2.5">
                          <Input
                            size="compact"
                            type="text"
                            value={editName}
                            onChange={(e) => setEditName(e.target.value)}
                            placeholder="Display name"
                            className="w-60"
                            autoFocus
                          />
                          <div className="flex flex-wrap items-center gap-1.5">
                            {SERVER_ACCENT_KEYS.map((key) => {
                              const hex = ACCENT_COLORS[key]
                              const picked = editAccent === key
                              return (
                                <button
                                  key={key}
                                  type="button"
                                  title={key}
                                  onClick={() => setEditAccent(key)}
                                  className={cn(
                                    'flex size-6 items-center justify-center rounded-full transition-transform',
                                    picked && 'ring-2 ring-foreground ring-offset-2 ring-offset-card scale-105',
                                  )}
                                  style={{ backgroundColor: hex }}
                                >
                                  {picked && <Check className="size-3 text-white" strokeWidth={2.5} />}
                                </button>
                              )
                            })}
                          </div>
                          <div className="ml-auto flex items-center gap-1.5">
                            <Button
                              type="button"
                              variant="primary"
                              size="compact"
                              leadingIcon={Check}
                              loading={savingServer}
                              disabled={!editName.trim()}
                              onClick={() => void saveEditServer(srv.id)}
                            >
                              Save
                            </Button>
                            <Button
                              type="button"
                              variant="ghost"
                              size="icon-compact"
                              aria-label="Cancel edit"
                              onClick={() => setEditingServerId(null)}
                            >
                              <X />
                            </Button>
                          </div>
                        </div>
                      </div>
                    )
                  }

                  return (
                    <div
                      key={srv.id}
                      className="flex flex-wrap items-center justify-between gap-3 px-5 py-3.5"
                    >
                      <div className="flex min-w-0 items-center gap-3.5">
                        <div
                          className="flex size-10 shrink-0 items-center justify-center rounded-xl text-[12.5px] font-semibold text-white shadow-[var(--shadow-2)]"
                          style={{ backgroundColor: color }}
                        >
                          {initials(srv.name)}
                        </div>
                        <div className="min-w-0">
                          <div className="flex flex-wrap items-center gap-2">
                            <span className="truncate text-[14px] font-semibold text-foreground">
                              {srv.name}
                            </span>
                            <Status tone={statusTone(srv.health)} size="compact" variant="dot">
                              {srv.health}
                            </Status>
                            {isActive && (
                              <Badge variant="solid" color="emerald" size="compact">
                                Active
                              </Badge>
                            )}
                            {srv.pinned_tls_sha256 && (
                              <Badge variant="solid" color="gray" size="compact">
                                TLS Pinned
                              </Badge>
                            )}
                          </div>
                          <div className="mt-0.5 truncate font-mono text-[12px] text-muted-foreground">
                            {srv.url}
                            {srv.user_email ? ` · ${srv.user_email}` : ''}
                            {srv.meta ? ` · v${srv.meta.server_version}` : ''}
                          </div>
                        </div>
                      </div>

                      <div className="flex flex-wrap items-center gap-1.5">
                        <Button
                          type="button"
                          variant="tertiary"
                          size="compact"
                          onClick={() => onSelectServer(srv.id)}
                        >
                          Open
                        </Button>
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon-compact"
                          disabled={idx === 0}
                          onClick={() => moveServer(idx, -1)}
                          title="Move up"
                          aria-label={`Move ${srv.name} up`}
                        >
                          <ArrowUp />
                        </Button>
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon-compact"
                          disabled={idx === servers.length - 1}
                          onClick={() => moveServer(idx, 1)}
                          title="Move down"
                          aria-label={`Move ${srv.name} down`}
                        >
                          <ArrowDown />
                        </Button>
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon-compact"
                          onClick={() => {
                            void navigator.clipboard.writeText(srv.url)
                            toast.success(`Copied ${srv.url}`)
                          }}
                          title="Copy server URL"
                          aria-label={`Copy ${srv.name} URL`}
                        >
                          <Copy />
                        </Button>
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon-compact"
                          onClick={() => beginEditServer(srv)}
                          title="Rename or change accent"
                          aria-label={`Edit ${srv.name}`}
                        >
                          <Pencil />
                        </Button>
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon-compact"
                          onClick={() => setConfirmRemove(srv)}
                          title="Remove server"
                          aria-label={`Remove ${srv.name}`}
                          className="text-muted-foreground hover:text-fail"
                        >
                          <Trash2 />
                        </Button>
                      </div>
                    </div>
                  )
                })}
              </div>
            )}
          </Panel>

          <Panel
            title={
              <>
                <ShieldCheck className="size-4 text-primary" />
                Local Storage & Security Boundary
              </>
            }
          >
            <dl className="grid grid-cols-1 gap-4 text-[13px] sm:grid-cols-2">
              <div>
                <dt className="text-[12px] text-muted-foreground">Server Registry File</dt>
                <dd className="mt-1 break-all font-mono text-[12px] text-foreground">
                  {appInfo?.registry_path ?? '—'}
                </dd>
              </div>
              <div>
                <dt className="text-[12px] text-muted-foreground">Device Token Storage</dt>
                <dd className="mt-1 text-foreground">
                  OS Keychain (never written to disk or exposed to the webview)
                </dd>
              </div>
            </dl>
          </Panel>
        </div>
      )}

      {/* CLI & MCP */}
      {tab === 'cli' && (
        <div className="space-y-5">
          <Panel
            title={
              <>
                <Terminal className="size-4 text-primary" />
                Rig CLI Status
              </>
            }
            actions={
              <Button
                type="button"
                variant="tertiary"
                size="compact"
                leadingIcon={RefreshCw}
                loading={appInfoQuery.isFetching}
                onClick={() => void appInfoQuery.refetch()}
              >
                Re-detect
              </Button>
            }
          >
            <dl className="grid grid-cols-1 gap-4 text-[13.5px] sm:grid-cols-2">
              <div>
                <dt className="text-[12px] text-muted-foreground">Binary Status</dt>
                <dd className="mt-1 flex items-center gap-2">
                  <Status tone={appInfo?.cli.installed ? 'ok' : 'idle'} size="compact" variant="dot">
                    {appInfo?.cli.installed ? 'Installed' : 'Not found on PATH'}
                  </Status>
                  {appInfo?.cli.version && (
                    <span className="font-mono text-[12.5px] text-foreground">
                      {appInfo.cli.version}
                    </span>
                  )}
                </dd>
              </div>
              <div>
                <dt className="text-[12px] text-muted-foreground">Binary Path</dt>
                <dd className="mt-1 break-all font-mono text-[12.5px] text-foreground">
                  {appInfo?.cli.binary_path ?? '—'}
                </dd>
              </div>
              <div>
                <dt className="text-[12px] text-muted-foreground">CLI Config File</dt>
                <dd className="mt-1 break-all font-mono text-[12.5px] text-foreground">
                  {appInfo?.cli.config_path ?? '~/.config/rig/config.toml'}
                  {appInfo?.cli.config_exists ? '' : ' (not created yet)'}
                </dd>
              </div>
              <div>
                <dt className="text-[12px] text-muted-foreground">Saved CLI Contexts</dt>
                <dd className="mt-1 font-mono text-[12.5px] text-foreground">
                  {appInfo?.cli.contexts_count ?? 0} context
                  {appInfo?.cli.contexts_count === 1 ? '' : 's'}
                  {appInfo?.cli.active_context ? ` · active: ${appInfo.cli.active_context}` : ''}
                </dd>
              </div>
            </dl>
          </Panel>

          <Panel
            title="Common Commands"
            description="Copy-ready commands for terminal workflows and local deployments."
          >
            <div className="space-y-2.5">
              <CopyableCodeRow
                label="Log CLI into your active server"
                command={
                  activeServer
                    ? `rig login ${activeServer.url}`
                    : 'rig login https://rig.example.com'
                }
              />
              <CopyableCodeRow
                label="Deploy current directory directly (delta zstd upload)"
                command="rig up"
              />
              <CopyableCodeRow
                label="Generate a one-time device pairing code"
                command="rig devices pair"
              />
              <CopyableCodeRow
                label="Install or upgrade Rig on a Linux host"
                command="curl -fsSL https://github.com/ksaurav24/rig/releases/latest/download/install.sh | sudo bash"
              />
            </div>
          </Panel>

          <Panel
            title="MCP Server Configuration"
            description="Connect Claude, Cursor, or T3 Code directly to your Rig servers via stdio MCP."
            actions={
              <Button
                type="button"
                variant="tertiary"
                size="compact"
                leadingIcon={Copy}
                onClick={() => {
                  void navigator.clipboard.writeText(mcpSnippet)
                  toast.success('Copied MCP config JSON')
                }}
              >
                Copy JSON
              </Button>
            }
          >
            <pre className="overflow-x-auto rounded-lg border border-border/80 bg-surface p-3.5 font-mono text-[12.5px] leading-relaxed text-foreground">
              {mcpSnippet}
            </pre>
          </Panel>
        </div>
      )}

      {/* VERSIONS & UPDATES */}
      {tab === 'updates' && (
        <div className="space-y-5">
          <Panel
            title="Rig Desktop Version & Updates"
            description="Check GitHub Releases for newer versions of Rig Desktop, CLI, and rig-server."
            actions={
              <div className="flex items-center gap-2">
                <Button
                  type="button"
                  variant="tertiary"
                  size="compact"
                  leadingIcon={ExternalLink}
                  onClick={() =>
                    void desktopApi.openExternalUrl(
                      updateResult?.release_url ?? 'https://github.com/ksaurav24/rig/releases',
                    )
                  }
                >
                  Releases
                </Button>
                <Button
                  type="button"
                  variant="primary"
                  size="compact"
                  leadingIcon={Download}
                  loading={checkingUpdates}
                  onClick={() => void runUpdateCheck()}
                >
                  Check for Updates
                </Button>
              </div>
            }
          >
            <dl className="grid grid-cols-1 gap-4 text-[13.5px] sm:grid-cols-3">
              <div>
                <dt className="text-[12px] text-muted-foreground">Rig Desktop</dt>
                <dd className="mt-1 font-mono font-semibold text-foreground">
                  v{appInfo?.app_version ?? perfQuery.data?.client_version ?? '0.1.0'}
                </dd>
              </div>
              <div>
                <dt className="text-[12px] text-muted-foreground">Platform</dt>
                <dd className="mt-1 font-mono text-foreground">
                  {appInfo ? `${appInfo.os} (${appInfo.arch})` : '—'}
                </dd>
              </div>
              <div>
                <dt className="text-[12px] text-muted-foreground">Session Uptime</dt>
                <dd className="mt-1 font-mono text-foreground tabular-nums">
                  {perfQuery.data ? formatDurationMs(perfQuery.data.uptime_ms) : '—'}
                </dd>
              </div>
            </dl>

            {updateResult && (
              <div className="mt-4">
                <Banner
                  status={
                    updateResult.error
                      ? 'warning'
                      : updateResult.update_available
                        ? 'info'
                        : 'success'
                  }
                  contrast="high"
                  size="compact"
                >
                  <BannerTitle>
                    {updateResult.error
                      ? 'Update check failed'
                      : updateResult.update_available
                        ? `Update available: v${updateResult.latest_version}`
                        : `You are up to date (v${updateResult.current_version})`}
                  </BannerTitle>
                  <BannerDescription>
                    {updateResult.error
                      ? updateResult.error
                      : updateResult.update_available
                        ? `Current v${updateResult.current_version} → Latest v${updateResult.latest_version}. Click Releases above to download.`
                        : `Checked ${relativeTime(updateResult.checked_at)}.`}
                  </BannerDescription>
                </Banner>
              </div>
            )}
          </Panel>

          <Panel
            title="Component & Server Version Matrix"
            description="Compare this desktop app against your local CLI and every paired Rig server."
            flush
          >
            <div className="divide-y divide-border/80 text-[13.5px]">
              <div className="flex items-center justify-between gap-3 px-5 py-3">
                <div>
                  <div className="font-medium text-foreground">Rig Desktop (this app)</div>
                  <div className="font-mono text-[12px] text-muted-foreground">
                    rig-client v{appInfo?.client_version ?? '0.1.0'}
                  </div>
                </div>
                <Badge variant="solid" color="emerald" size="compact" className="font-mono">
                  v{appInfo?.app_version ?? '0.1.0'}
                </Badge>
              </div>

              <div className="flex items-center justify-between gap-3 px-5 py-3">
                <div>
                  <div className="font-medium text-foreground">Rig CLI (local)</div>
                  <div className="font-mono text-[12px] text-muted-foreground">
                    {appInfo?.cli.binary_path ?? 'Not installed on PATH'}
                  </div>
                </div>
                <Badge
                  variant="solid"
                  color={appInfo?.cli.installed ? 'gray' : 'amber'}
                  size="compact"
                  className="font-mono"
                >
                  {appInfo?.cli.version ?? 'Not installed'}
                </Badge>
              </div>

              {servers.map((s) => {
                const ver = s.meta?.server_version
                const matches = Boolean(ver && appInfo && ver === appInfo.app_version)
                return (
                  <div key={s.id} className="flex items-center justify-between gap-3 px-5 py-3">
                    <div className="min-w-0">
                      <div className="truncate font-medium text-foreground">{s.name}</div>
                      <div className="truncate font-mono text-[12px] text-muted-foreground">
                        {s.url}
                        {s.meta?.mode ? ` · ${s.meta.mode}` : ''}
                        {s.meta?.api_version ? ` · API v${s.meta.api_version}` : ''}
                      </div>
                    </div>
                    <Badge
                      variant="solid"
                      color={!ver ? 'gray' : matches ? 'emerald' : 'amber'}
                      size="compact"
                      className="shrink-0 font-mono"
                    >
                      {ver ? `v${ver}` : 'Unknown'}
                    </Badge>
                  </div>
                )
              })}
            </div>
          </Panel>

          {perfQuery.data && (
            <Panel
              title="Runtime Diagnostics"
              description="Native Rust log ring buffers and live connection state."
            >
              <div className="flex flex-wrap items-center gap-6 font-mono text-[12.5px] text-muted-foreground">
                <span>
                  Paired servers:{' '}
                  <strong className="text-foreground">{perfQuery.data.server_count}</strong>
                </span>
                <span>
                  Active log rings:{' '}
                  <strong className="text-foreground">{perfQuery.data.log_buffers.length}</strong>
                </span>
                <span>
                  Uptime:{' '}
                  <strong className="text-foreground">
                    {formatDurationMs(perfQuery.data.uptime_ms)}
                  </strong>
                </span>
              </div>
            </Panel>
          )}
        </div>
      )}

      <ConfirmDialog
        open={Boolean(confirmRemove)}
        onOpenChange={(open) => {
          if (!open) setConfirmRemove(null)
        }}
        title="Disconnect Server"
        description="Removes this server from Rig Desktop and deletes its device token from your OS keychain. Running apps on the server are unaffected."
        target={confirmRemove ? `${confirmRemove.name} (${confirmRemove.url})` : undefined}
        confirmLabel="Remove Server"
        tone="danger"
        onConfirm={() => {
          if (confirmRemove) onRemoveServer(confirmRemove.id)
        }}
      />
    </Page>
  )
}
