import { useQuery, useQueryClient } from '@tanstack/react-query'
import {
  Box,
  ChevronRight,
  Database,
  FolderGit2,
  LayoutDashboard,
  Minus,
  PanelLeftClose,
  PanelLeftOpen,
  Plus,
  Rocket,
  Search,
  Server,
  Settings,
} from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState, type ComponentType } from 'react'
import { toast } from 'sonner'
import { AddServerModal } from './components/AddServerModal'
import { AppsView, type AppDetailTab } from './components/AppsView'
import { CommandPalette } from './components/CommandPalette'
import { DatabasesView } from './components/DatabasesView'
import { DeployModal } from './components/DeployModal'
import { DesktopSettingsView, type DesktopSettingsTab } from './components/DesktopSettingsView'
import { LocalReposView } from './components/LocalReposView'
import { OverviewView } from './components/OverviewView'
import { ServerRail } from './components/ServerRail'
import { SettingsView } from './components/SettingsView'
import { Logo } from '@rig/ui/app/logo'
import { Button } from '@rig/ui/button'
import { FluidHoverHighlight } from '@rig/ui/fluid-hover-highlight'
import { Tooltip, TooltipGroup } from '@rig/ui/tooltip'
import { useFluidHover, useRegisterFluidHoverItem } from '@rig/ui/hooks/use-fluid-hover'
import { desktopApi } from './lib/ipc'
import { KeyCaps } from '@rig/ui/lib/key-cap'
import { MAX_ZOOM, MIN_ZOOM, usePreferences } from './lib/preferences'
import { useShape } from '@rig/ui/lib/shape-context'
import type { ServerView } from './lib/types'
import { cn } from '@rig/ui/lib/utils'

type Section = 'overview' | 'apps' | 'local' | 'databases' | 'settings'

interface NavItemDef {
  id: Section
  label: string
  icon: ComponentType<{ className?: string; size?: number; strokeWidth?: number }>
  badge?: number
}

function SidebarNavItem({
  item,
  index,
  active,
  compact,
  registerItem,
  onClick,
}: {
  item: NavItemDef
  index: number
  active: boolean
  compact: boolean
  registerItem: (index: number, el: HTMLElement | null) => void
  onClick: () => void
}) {
  const ref = useRef<HTMLButtonElement>(null)
  const shape = useShape()
  const Icon = item.icon
  useRegisterFluidHoverItem(registerItem, index, ref)

  const button = (
    <button
      ref={ref}
      type="button"
      data-fluid-hover-index={index}
      onClick={onClick}
      aria-current={active ? 'page' : undefined}
      className={cn(
        'relative z-10 flex items-center text-[13.5px] transition-colors duration-80 outline-none select-none',
        'focus-visible:ring-1 focus-visible:ring-[color:var(--focus-ring)]',
        shape.item,
        compact ? 'size-10 justify-center' : 'h-9 w-full gap-2.5 px-3',
        active
          ? 'bg-active font-medium text-foreground'
          : 'text-sidebar-foreground hover:text-foreground',
      )}
    >
      <Icon
        size={17}
        strokeWidth={active ? 2 : 1.65}
        className={cn('shrink-0 transition-colors', active ? 'text-primary' : 'text-muted-foreground')}
      />
      {!compact && <span className="min-w-0 flex-1 truncate text-left">{item.label}</span>}
      {!compact && item.badge !== undefined && (
        <span className="font-mono text-[11.5px] text-muted-foreground tabular-nums">
          {item.badge}
        </span>
      )}
    </button>
  )

  if (compact) {
    return (
      <Tooltip side="right" content={item.label}>
        {button}
      </Tooltip>
    )
  }
  return button
}

export default function App() {
  const qc = useQueryClient()
  const { prefs, resolvedTheme, updatePrefs, zoomIn, zoomOut, resetZoom } = usePreferences()
  const shape = useShape()
  const [section, setSection] = useState<Section>(() => prefs.defaultSection)
  const [appSettingsOpen, setAppSettingsOpen] = useState(false)
  const [appSettingsTab, setAppSettingsTab] = useState<DesktopSettingsTab>('general')
  const [selectedAppId, setSelectedAppId] = useState<string | null>(null)
  const [appTab, setAppTab] = useState<AppDetailTab>('deployments')
  const [addOpen, setAddOpen] = useState(false)
  const [paletteOpen, setPaletteOpen] = useState(false)
  const [autoOpenCreateDb, setAutoOpenCreateDb] = useState(false)
  const [deployModal, setDeployModal] = useState<{
    open: boolean
    initialLocalPath?: string | null
    initialName?: string | null
  }>({ open: false })

  const notificationsEnabledRef = useRef(prefs.notificationsEnabled)
  notificationsEnabledRef.current = prefs.notificationsEnabled

  const navListRef = useRef<HTMLDivElement>(null)
  const navHover = useFluidHover(navListRef)

  const serversQuery = useQuery({
    queryKey: ['desktop', 'servers'],
    queryFn: () => desktopApi.listServers(),
  })

  const servers = serversQuery.data?.servers ?? []
  const activeServerId = serversQuery.data?.active_server_id ?? servers[0]?.id ?? null
  const activeServer: ServerView | null = useMemo(
    () => servers.find((s) => s.id === activeServerId) ?? null,
    [activeServerId, servers],
  )

  const snapshotQuery = useQuery({
    queryKey: ['desktop', 'snapshot', activeServerId],
    queryFn: () => desktopApi.getServerSnapshot(activeServerId!),
    enabled: Boolean(activeServerId),
    placeholderData: () => {
      if (!activeServer) return undefined
      return {
        server_id: activeServer.id,
        from_cache: true,
        system: activeServer.cache.system ?? null,
        projects: activeServer.cache.projects ?? [],
        apps: activeServer.cache.apps ?? [],
        databases: activeServer.cache.databases ?? [],
        recent_events: [],
        meta: activeServer.meta ?? null,
        fetch_ms: 0,
      }
    },
  })

  // Subscribe to Rust ConnectionManager push events (health dots + real-time server events).
  useEffect(() => {
    let unlistenStatus: (() => void) | undefined
    let unlistenEvent: (() => void) | undefined

    void desktopApi
      .onServerStatus(({ server_id, health, error }) => {
        qc.setQueryData<{ active_server_id?: string | null; servers: ServerView[] }>(
          ['desktop', 'servers'],
          (prev) => {
            if (!prev) return prev
            return {
              ...prev,
              servers: prev.servers.map((s) =>
                s.id === server_id ? { ...s, health, health_error: error } : s,
              ),
            }
          },
        )
      })
      .then((fn) => {
        unlistenStatus = fn
      })

    void desktopApi
      .onServerEvent(({ server_id, server_name, event }) => {
        if (notificationsEnabledRef.current) {
          if (
            event.kind === 'deploy.failed' ||
            event.kind === 'app.crashed' ||
            event.kind === 'app.down'
          ) {
            toast.error(`${server_name}: ${event.title}`, {
              description: event.detail ?? undefined,
            })
          } else if (event.kind === 'deploy.finished') {
            toast.success(`${server_name}: ${event.title}`)
          }
        }
        void qc.invalidateQueries({ queryKey: ['desktop', 'snapshot', server_id] })
      })
      .then((fn) => {
        unlistenEvent = fn
      })

    return () => {
      unlistenStatus?.()
      unlistenEvent?.()
    }
  }, [qc])

  const openAppSettings = useCallback((tab: DesktopSettingsTab = 'general') => {
    setAppSettingsTab(tab)
    setAppSettingsOpen(true)
  }, [])

  const handleSelectServer = useCallback(
    async (id: string) => {
      setAppSettingsOpen(false)
      if (id === activeServerId) return
      qc.setQueryData<{ active_server_id?: string | null; servers: ServerView[] }>(
        ['desktop', 'servers'],
        (prev) => (prev ? { ...prev, active_server_id: id } : prev),
      )
      setSelectedAppId(null)
      await desktopApi.selectServer(id)
    },
    [activeServerId, qc],
  )

  const handleRemoveServer = useCallback(
    async (id: string) => {
      try {
        await desktopApi.removeServer(id)
        toast.success('Server removed from Rig Desktop')
        setSelectedAppId(null)
        await qc.invalidateQueries({ queryKey: ['desktop', 'servers'] })
      } catch (e) {
        toast.error(String(e))
      }
    },
    [qc],
  )

  const handleReorderServers = useCallback(
    async (orderedIds: string[]) => {
      qc.setQueryData<{ active_server_id?: string | null; servers: ServerView[] }>(
        ['desktop', 'servers'],
        (prev) => {
          if (!prev) return prev
          const byId = new Map(prev.servers.map((s) => [s.id, s]))
          const next = orderedIds
            .map((id) => byId.get(id))
            .filter((s): s is ServerView => Boolean(s))
          return { ...prev, servers: next }
        },
      )
      await desktopApi.reorderServers(orderedIds)
    },
    [qc],
  )

  // Global keyboard shortcuts & Ctrl+Wheel zoom:
  // Cmd/Ctrl+K (palette), Cmd/Ctrl+1..9 (server switch), Cmd/Ctrl+N (add server),
  // Cmd/Ctrl+=/+/-/0 (zoom in/out/reset), Ctrl+Wheel (smooth zoom).
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey
      if (!mod) return
      const key = e.key.toLowerCase()
      if (key === 'k') {
        e.preventDefault()
        setPaletteOpen((v) => !v)
      } else if (key === ',') {
        e.preventDefault()
        setAppSettingsOpen((v) => !v)
      } else if (key === 'n') {
        e.preventDefault()
        setAddOpen(true)
      } else if (e.key === '=' || e.key === '+' || e.code === 'Equal' || e.code === 'NumpadAdd') {
        e.preventDefault()
        zoomIn()
      } else if (e.key === '-' || e.key === '_' || e.code === 'Minus' || e.code === 'NumpadSubtract') {
        e.preventDefault()
        zoomOut()
      } else if (e.key === '0' || e.code === 'Digit0' || e.code === 'Numpad0') {
        if (!e.shiftKey) {
          e.preventDefault()
          resetZoom()
        }
      } else if (key >= '1' && key <= '9') {
        const idx = Number(key) - 1
        if (servers[idx]) {
          e.preventDefault()
          void handleSelectServer(servers[idx].id)
        }
      }
    }

    const onWheel = (e: WheelEvent) => {
      if (!e.ctrlKey && !e.metaKey) return
      e.preventDefault()
      if (e.deltaY < 0) zoomIn()
      else if (e.deltaY > 0) zoomOut()
    }

    window.addEventListener('keydown', onKeyDown)
    window.addEventListener('wheel', onWheel, { passive: false })
    return () => {
      window.removeEventListener('keydown', onKeyDown)
      window.removeEventListener('wheel', onWheel)
    }
  }, [handleSelectServer, resetZoom, servers, zoomIn, zoomOut])

  const openDeployModal = useCallback((initialLocalPath?: string, initialName?: string) => {
    setDeployModal({ open: true, initialLocalPath: initialLocalPath ?? null, initialName: initialName ?? null })
  }, [])

  const openCreateDatabase = useCallback(() => {
    setSelectedAppId(null)
    setSection('databases')
    setAutoOpenCreateDb(true)
  }, [])

  const navItems: NavItemDef[] = [
    { id: 'overview', label: 'Overview', icon: LayoutDashboard },
    {
      id: 'apps',
      label: 'Apps',
      icon: Box,
      badge: snapshotQuery.data?.apps.length,
    },
    {
      id: 'local',
      label: 'Local Repos',
      icon: FolderGit2,
    },
    {
      id: 'databases',
      label: 'Databases',
      icon: Database,
      badge: snapshotQuery.data?.databases.length,
    },
    { id: 'settings', label: 'Settings', icon: Settings },
  ]

  const compactNav = prefs.sidebarLayout === 'compact'
  const selectedAppName = useMemo(
    () => snapshotQuery.data?.apps.find((a) => a.id === selectedAppId)?.name ?? null,
    [selectedAppId, snapshotQuery.data?.apps],
  )
  const zoomPercent = Math.round(prefs.zoom * 100)

  const sectionLabel =
    section === 'local'
      ? 'Local Repos'
      : section === 'overview'
        ? 'Overview'
        : section === 'apps'
          ? 'Applications'
          : section === 'databases'
            ? 'Databases'
            : 'Settings'

  return (
    <div className="flex h-screen w-screen overflow-hidden bg-background text-foreground">
      {/* Multi-Server Switcher Rail */}
      <ServerRail
        servers={servers}
        activeId={activeServerId}
        appSettingsOpen={appSettingsOpen}
        onSelect={(id) => void handleSelectServer(id)}
        onAdd={() => setAddOpen(true)}
        onReorder={(ids) => void handleReorderServers(ids)}
        onOpenAppSettings={() => openAppSettings('general')}
        theme={resolvedTheme}
        onToggleTheme={() => updatePrefs({ theme: resolvedTheme === 'dark' ? 'light' : 'dark' })}
      />

      {/* Active Server Navigation Sidebar */}
      {activeServer && !appSettingsOpen && (
        <nav
          aria-label="Server navigation"
          className={cn(
            'flex shrink-0 flex-col border-r border-sidebar-border bg-sidebar transition-[width] duration-150',
            compactNav ? 'w-[64px] items-center' : 'w-60',
          )}
        >
          <div className={cn('border-b border-sidebar-border/80', compactNav ? 'flex flex-col items-center gap-2 p-2.5' : 'px-3.5 py-3')}>
            {compactNav ? (
              <>
                <Tooltip side="right" content="Expand sidebar">
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    onClick={() => updatePrefs({ sidebarLayout: 'expanded' })}
                    aria-label="Expand sidebar"
                    className="size-10 text-muted-foreground hover:text-foreground"
                  >
                    <PanelLeftOpen className="size-4" />
                  </Button>
                </Tooltip>
                <Tooltip side="right" content="Deploy App">
                  <Button
                    type="button"
                    variant="primary"
                    size="icon"
                    onClick={() => openDeployModal()}
                    aria-label="Deploy App"
                    className="size-10"
                  >
                    <Rocket className="size-4" />
                  </Button>
                </Tooltip>
              </>
            ) : (
              <>
                <div className="mb-3 flex items-center justify-between gap-2">
                  <div className="flex min-w-0 items-center gap-2">
                    <span
                      aria-hidden="true"
                      className={cn(
                        'size-2 shrink-0 rounded-full',
                        activeServer.health === 'online'
                          ? 'bg-ok'
                          : activeServer.health === 'deploying'
                            ? 'bg-warn animate-pulse'
                            : activeServer.health === 'connecting'
                              ? 'bg-busy animate-pulse'
                              : activeServer.health === 'degraded'
                                ? 'bg-fail'
                                : 'bg-idle',
                      )}
                    />
                    <span className="truncate text-[14.5px] font-semibold tracking-tight text-foreground">
                      {activeServer.name}
                    </span>
                  </div>
                  <Tooltip side="right" content="Collapse sidebar">
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon-compact"
                      onClick={() => updatePrefs({ sidebarLayout: 'compact' })}
                      aria-label="Collapse sidebar"
                      className="shrink-0 text-muted-foreground hover:text-foreground"
                    >
                      <PanelLeftClose />
                    </Button>
                  </Tooltip>
                </div>
                <Button
                  type="button"
                  variant="primary"
                  leadingIcon={Rocket}
                  onClick={() => openDeployModal()}
                  className="w-full"
                >
                  Deploy App
                </Button>
              </>
            )}
          </div>

          <TooltipGroup>
            <div
              ref={navListRef}
              onMouseEnter={navHover.handlers.onMouseEnter}
              onMouseMove={navHover.handlers.onMouseMove}
              onMouseLeave={navHover.handlers.onMouseLeave}
              onClick={navHover.handlers.onClick}
              className={cn(
                'relative flex-1 space-y-1 overflow-y-auto p-2.5',
                compactNav && 'flex flex-col items-center',
              )}
            >
              <FluidHoverHighlight hover={navHover} className={cn('z-0', shape.item)} />
              {navItems.map((item, idx) => (
                <SidebarNavItem
                  key={item.id}
                  item={item}
                  index={idx}
                  active={section === item.id}
                  compact={compactNav}
                  registerItem={navHover.registerItem}
                  onClick={() => {
                    setSection(item.id)
                    if (item.id !== 'apps') setSelectedAppId(null)
                  }}
                />
              ))}
            </div>
          </TooltipGroup>

          {!compactNav && prefs.showShortcutHints && (
            <div className="border-t border-sidebar-border/80 px-4 py-3">
              <div className="space-y-1.5 text-[12px] text-muted-foreground">
                <div className="flex items-center justify-between">
                  <span>Command menu</span>
                  <KeyCaps keys={['Ctrl', 'K']} />
                </div>
                <div className="flex items-center justify-between">
                  <span>Switch server</span>
                  <KeyCaps keys={['Ctrl', '1-9']} />
                </div>
                <div className="flex items-center justify-between">
                  <span>App settings</span>
                  <KeyCaps keys={['Ctrl', ',']} />
                </div>
                <div className="flex items-center justify-between">
                  <span>Zoom UI</span>
                  <KeyCaps keys={['Ctrl', '+/-']} />
                </div>
              </div>
            </div>
          )}
        </nav>
      )}

      {/* Main Content Area */}
      <div className="flex min-w-0 flex-1 flex-col overflow-hidden bg-background">
        <header className="flex h-12 shrink-0 items-center justify-between gap-3 border-b border-border/80 bg-surface/70 px-5">
          {/* Interactive Breadcrumb */}
          <div className="flex min-w-0 items-center gap-1.5 text-[13.5px] text-muted-foreground">
            {appSettingsOpen ? (
              <>
                <button
                  type="button"
                  onClick={() => setAppSettingsOpen(false)}
                  className="truncate transition-colors hover:text-foreground"
                >
                  {activeServer ? activeServer.name : 'Rig Desktop'}
                </button>
                <ChevronRight className="size-3.5 shrink-0 text-muted-foreground/60" />
                <span className="truncate font-medium text-foreground">App Settings</span>
              </>
            ) : (
              <>
                <button
                  type="button"
                  onClick={() => {
                    setSelectedAppId(null)
                    setSection('overview')
                  }}
                  className="truncate font-medium text-foreground transition-colors hover:text-primary"
                >
                  {activeServer ? activeServer.name : 'Rig Desktop'}
                </button>
                {activeServer && (
                  <>
                    <ChevronRight className="size-3.5 shrink-0 text-muted-foreground/60" />
                    <button
                      type="button"
                      onClick={() => setSelectedAppId(null)}
                      className={cn(
                        'truncate transition-colors',
                        selectedAppName
                          ? 'hover:text-foreground'
                          : 'font-medium text-foreground',
                      )}
                    >
                      {sectionLabel}
                    </button>
                    {selectedAppName && (
                      <>
                        <ChevronRight className="size-3.5 shrink-0 text-muted-foreground/60" />
                        <span className="truncate font-medium text-foreground">{selectedAppName}</span>
                      </>
                    )}
                  </>
                )}
              </>
            )}
          </div>

          <div className="flex shrink-0 items-center gap-2">
            {/* Quick Zoom Control — h-7 aligns flush with compact header buttons */}
            <div
              className={cn(
                'inline-flex h-7 items-center border border-border bg-card/70 px-0.5 text-[12px] text-muted-foreground',
                shape.button,
              )}
              title="UI Zoom (Ctrl+ / Ctrl- / Ctrl+0)"
            >
              <button
                type="button"
                onClick={zoomOut}
                disabled={prefs.zoom <= MIN_ZOOM}
                aria-label="Zoom out"
                className={cn(
                  'flex size-5.5 items-center justify-center transition-colors hover:bg-hover hover:text-foreground disabled:opacity-35',
                  shape.item,
                )}
              >
                <Minus className="size-3" />
              </button>
              <button
                type="button"
                onClick={resetZoom}
                title="Reset zoom to 100% (Ctrl+0)"
                className={cn(
                  'min-w-[42px] px-1.5 font-mono text-[11.5px] leading-none tabular-nums transition-colors hover:bg-hover hover:text-foreground',
                  prefs.zoom !== 1 ? 'font-medium text-primary' : 'text-muted-foreground',
                  shape.item,
                )}
              >
                {zoomPercent}%
              </button>
              <button
                type="button"
                onClick={zoomIn}
                disabled={prefs.zoom >= MAX_ZOOM}
                aria-label="Zoom in"
                className={cn(
                  'flex size-5.5 items-center justify-center transition-colors hover:bg-hover hover:text-foreground disabled:opacity-35',
                  shape.item,
                )}
              >
                <Plus className="size-3" />
              </button>
            </div>

            <Button
              type="button"
              variant="tertiary"
              size="compact"
              leadingIcon={Search}
              shortcut={['Ctrl', 'K']}
              onClick={() => setPaletteOpen(true)}
            >
              Jump to…
            </Button>
          </div>
        </header>

        <main className="flex min-h-0 flex-1 flex-col overflow-y-auto">
          {appSettingsOpen ? (
            <DesktopSettingsView
              key={appSettingsTab}
              initialTab={appSettingsTab}
              servers={servers}
              activeServerId={activeServerId}
              onSelectServer={(id) => void handleSelectServer(id)}
              onAddServer={() => setAddOpen(true)}
              onRemoveServer={(id) => void handleRemoveServer(id)}
              onReorderServers={(ids) => void handleReorderServers(ids)}
            />
          ) : !activeServer ? (
            <div className="mx-auto flex max-w-lg flex-1 flex-col items-center justify-center px-6 py-12 text-center">
              <div className="mb-5 flex size-16 items-center justify-center rounded-2xl bg-card shadow-[var(--shadow-4)]">
                <Logo className="size-10" />
              </div>
              <h1 className="text-2xl font-semibold tracking-tight text-foreground">
                Control all your Rig servers in one place
              </h1>
              <p className="mt-2.5 text-[14px] leading-relaxed text-muted-foreground">
                Pair headless or full Rig servers over public HTTPS, LAN (pinned internal CA), or
                Tailscale. Tokens stay in your OS keychain and all streaming runs natively in Rust.
              </p>
              <div className="mt-7 flex flex-wrap items-center justify-center gap-3">
                <Button
                  type="button"
                  variant="primary"
                  leadingIcon={Server}
                  onClick={() => setAddOpen(true)}
                >
                  Add Rig Server
                </Button>
                <Button
                  type="button"
                  variant="tertiary"
                  leadingIcon={Settings}
                  onClick={() => openAppSettings('general')}
                >
                  App Settings
                </Button>
              </div>
            </div>
          ) : (
            <>
              {section === 'overview' && (
                <OverviewView
                  server={activeServer}
                  snapshot={snapshotQuery.data}
                  loading={snapshotQuery.isFetching}
                  onRefresh={() => void snapshotQuery.refetch()}
                  onOpenApp={(appId) => {
                    setSelectedAppId(appId)
                    setAppTab('deployments')
                    setSection('apps')
                  }}
                  onDeployApp={() => openDeployModal()}
                  onOpenLocalRepos={() => {
                    setSelectedAppId(null)
                    setSection('local')
                  }}
                  onOpenDatabases={() => {
                    setSelectedAppId(null)
                    setSection('databases')
                  }}
                  onCreateDatabase={openCreateDatabase}
                />
              )}
              {section === 'apps' && (
                <AppsView
                  key={activeServer.id}
                  serverId={activeServer.id}
                  projects={snapshotQuery.data?.projects ?? []}
                  apps={snapshotQuery.data?.apps ?? []}
                  selectedAppId={selectedAppId}
                  activeTab={appTab}
                  onSelectApp={setSelectedAppId}
                  onSelectTab={setAppTab}
                  onRefreshServer={() => void snapshotQuery.refetch()}
                  onDeployNew={() => openDeployModal()}
                />
              )}
              {section === 'local' && (
                <LocalReposView
                  serverName={activeServer.name}
                  onDeployRepo={(path, name) => openDeployModal(path, name)}
                />
              )}
              {section === 'databases' && (
                <DatabasesView
                  key={activeServer.id}
                  serverId={activeServer.id}
                  projects={snapshotQuery.data?.projects ?? []}
                  databases={snapshotQuery.data?.databases ?? []}
                  autoOpenCreate={autoOpenCreateDb}
                  onConsumeAutoCreate={() => setAutoOpenCreateDb(false)}
                  onRefresh={() => void snapshotQuery.refetch()}
                />
              )}
              {section === 'settings' && (
                <SettingsView
                  key={activeServer.id}
                  server={activeServer}
                  onRemoveServer={(id) => void handleRemoveServer(id)}
                />
              )}
            </>
          )}
        </main>
      </div>

      <AddServerModal
        open={addOpen}
        onClose={() => setAddOpen(false)}
        onAdded={() => {
          setAppSettingsOpen(false)
          setSection(prefs.defaultSection)
          void qc.invalidateQueries({ queryKey: ['desktop', 'servers'] })
        }}
      />

      {activeServer && (
        <DeployModal
          open={deployModal.open}
          serverId={activeServer.id}
          projects={snapshotQuery.data?.projects ?? []}
          initialLocalPath={deployModal.initialLocalPath}
          initialName={deployModal.initialName}
          onClose={() => setDeployModal({ open: false })}
          onDeployed={(app) => {
            void snapshotQuery.refetch()
            setAppSettingsOpen(false)
            setSelectedAppId(app.id)
            setAppTab('deployments')
            setSection('apps')
          }}
        />
      )}

      <CommandPalette
        open={paletteOpen}
        onClose={() => setPaletteOpen(false)}
        servers={servers}
        activeServer={activeServer}
        apps={snapshotQuery.data?.apps ?? []}
        databases={snapshotQuery.data?.databases ?? []}
        onSelectServer={(id) => void handleSelectServer(id)}
        onNavigate={(sec, appId, tab) => {
          setAppSettingsOpen(false)
          setSection(sec)
          if (appId) setSelectedAppId(appId)
          if (tab) setAppTab(tab as AppDetailTab)
        }}
        onAddServer={() => setAddOpen(true)}
        onDeployApp={() => openDeployModal()}
        onCreateDatabase={openCreateDatabase}
        onOpenAppSettings={openAppSettings}
      />
    </div>
  )
}
