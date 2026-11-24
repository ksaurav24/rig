import { useQuery, useQueryClient } from '@tanstack/react-query'
import {
  Box,
  Command,
  Database,
  FolderGit2,
  LayoutDashboard,
  Palette,
  Plus,
  Rocket,
  Server,
  Settings,
} from 'lucide-react'
import { useCallback, useEffect, useMemo, useState } from 'react'
import { toast } from 'sonner'
import { AddServerModal } from './components/AddServerModal'
import { AppearanceModal } from './components/AppearancePanel'
import { AppsView, type AppDetailTab } from './components/AppsView'
import { CommandPalette } from './components/CommandPalette'
import { DatabasesView } from './components/DatabasesView'
import { DeployModal } from './components/DeployModal'
import { LocalReposView } from './components/LocalReposView'
import { OverviewView } from './components/OverviewView'
import { ServerRail } from './components/ServerRail'
import { SettingsView } from './components/SettingsView'
import { desktopApi } from './lib/ipc'
import { usePreferences } from './lib/preferences'
import type { ServerView } from './lib/types'

type Section = 'overview' | 'apps' | 'local' | 'databases' | 'settings'

export default function App() {
  const qc = useQueryClient()
  const { prefs, resolvedTheme, updatePrefs } = usePreferences()
  const [section, setSection] = useState<Section>(() => prefs.defaultSection)
  const [selectedAppId, setSelectedAppId] = useState<string | null>(null)
  const [appTab, setAppTab] = useState<AppDetailTab>('deployments')
  const [addOpen, setAddOpen] = useState(false)
  const [paletteOpen, setPaletteOpen] = useState(false)
  const [appearanceOpen, setAppearanceOpen] = useState(false)
  const [autoOpenCreateDb, setAutoOpenCreateDb] = useState(false)
  const [deployModal, setDeployModal] = useState<{
    open: boolean
    initialLocalPath?: string | null
    initialName?: string | null
  }>({ open: false })

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
        if (event.kind === 'deploy.failed' || event.kind === 'app.crashed' || event.kind === 'app.down') {
          toast.error(`${server_name}: ${event.title}`, {
            description: event.detail ?? undefined,
          })
        } else if (event.kind === 'deploy.finished') {
          toast.success(`${server_name}: ${event.title}`)
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

  const handleSelectServer = useCallback(
    async (id: string) => {
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

  // Global keyboard shortcuts: Cmd/Ctrl+K (palette), Cmd/Ctrl+1..9 (server switch), Cmd/Ctrl+N (add server).
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey
      if (!mod) return
      const key = e.key.toLowerCase()
      if (key === 'k') {
        e.preventDefault()
        setPaletteOpen((v) => !v)
      } else if (key === 'n') {
        e.preventDefault()
        setAddOpen(true)
      } else if (key >= '1' && key <= '9') {
        const idx = Number(key) - 1
        if (servers[idx]) {
          e.preventDefault()
          void handleSelectServer(servers[idx].id)
        }
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [handleSelectServer, servers])

  const openDeployModal = useCallback((initialLocalPath?: string, initialName?: string) => {
    setDeployModal({ open: true, initialLocalPath: initialLocalPath ?? null, initialName: initialName ?? null })
  }, [])

  const openCreateDatabase = useCallback(() => {
    setSelectedAppId(null)
    setSection('databases')
    setAutoOpenCreateDb(true)
  }, [])

  const navItems: { id: Section; label: string; icon: React.ReactNode; badge?: number }[] = [
    { id: 'overview', label: 'Overview', icon: <LayoutDashboard className="size-4" /> },
    {
      id: 'apps',
      label: 'Apps',
      icon: <Box className="size-4" />,
      badge: snapshotQuery.data?.apps.length,
    },
    {
      id: 'local',
      label: 'Local Repos',
      icon: <FolderGit2 className="size-4" />,
    },
    {
      id: 'databases',
      label: 'Databases',
      icon: <Database className="size-4" />,
      badge: snapshotQuery.data?.databases.length,
    },
    { id: 'settings', label: 'Settings', icon: <Settings className="size-4" /> },
  ]

  const compactNav = prefs.sidebarLayout === 'compact'

  return (
    <div className="flex h-screen w-screen overflow-hidden bg-bg text-fg">
      {/* Multi-Server Switcher Rail */}
      <ServerRail
        servers={servers}
        activeId={activeServerId}
        onSelect={(id) => void handleSelectServer(id)}
        onAdd={() => setAddOpen(true)}
        onReorder={(ids) => void handleReorderServers(ids)}
        onOpenAppearance={() => setAppearanceOpen(true)}
        theme={resolvedTheme}
        onToggleTheme={() => updatePrefs({ theme: resolvedTheme === 'dark' ? 'light' : 'dark' })}
      />

      {/* Active Server Navigation Sidebar */}
      {activeServer && (
        <nav
          aria-label="Server navigation"
          className={`flex shrink-0 flex-col border-r border-line bg-surface/60 transition-[width] duration-150 ${
            compactNav ? 'w-16 items-center' : 'w-52'
          }`}
        >
          <div className={`border-b border-line ${compactNav ? 'p-2' : 'px-3.5 py-3'}`}>
            {!compactNav && (
              <>
                <div className="truncate text-xs font-semibold text-fg">{activeServer.name}</div>
                <div className="mt-0.5 truncate font-mono text-[10px] text-muted">
                  {activeServer.user_email ?? activeServer.url}
                </div>
              </>
            )}
            <button
              type="button"
              onClick={() => openDeployModal()}
              title="Deploy App"
              className={`flex items-center justify-center gap-1.5 rounded-lg bg-emerald-600 text-xs font-medium text-white hover:bg-emerald-500 ${
                compactNav ? 'size-10' : 'mt-2.5 w-full px-2.5 py-1.5'
              }`}
            >
              <Rocket className="size-3.5" />
              {!compactNav && 'Deploy App'}
            </button>
          </div>
          <div className={`flex-1 space-y-0.5 p-2 ${compactNav ? 'flex flex-col items-center' : ''}`}>
            {navItems.map((item) => {
              const active = section === item.id
              return (
                <button
                  key={item.id}
                  type="button"
                  title={compactNav ? item.label : undefined}
                  onClick={() => {
                    setSection(item.id)
                    if (item.id !== 'apps') setSelectedAppId(null)
                  }}
                  className={`flex items-center rounded-lg text-xs font-medium transition-colors ${
                    compactNav
                      ? 'relative size-10 justify-center'
                      : 'w-full gap-2.5 px-2.5 py-2'
                  } ${
                    active
                      ? 'bg-surface-2 text-fg ring-1 ring-line'
                      : 'text-muted hover:bg-surface-2/60 hover:text-fg'
                  }`}
                >
                  {item.icon}
                  {!compactNav && <span className="flex-1 text-left">{item.label}</span>}
                  {!compactNav && item.badge !== undefined && (
                    <span className="font-mono text-[10px] text-muted tabular-nums">
                      {item.badge}
                    </span>
                  )}
                </button>
              )
            })}
          </div>
          {!compactNav && prefs.showShortcutHints && (
            <div className="border-t border-line p-2.5 text-[10px] text-muted">
              <div className="flex items-center justify-between">
                <span>Command Palette</span>
                <kbd className="rounded bg-bg px-1.5 py-0.5 font-mono ring-1 ring-line">Ctrl K</kbd>
              </div>
              <div className="mt-1 flex items-center justify-between">
                <span>Switch Server</span>
                <kbd className="rounded bg-bg px-1.5 py-0.5 font-mono ring-1 ring-line">Ctrl 1-9</kbd>
              </div>
            </div>
          )}
        </nav>
      )}

      {/* Main Content Area */}
      <div className="flex min-w-0 flex-1 flex-col overflow-hidden">
        <header className="flex h-11 shrink-0 items-center justify-between border-b border-line bg-surface/50 px-4">
          <div className="flex items-center gap-2 text-xs text-muted">
            <span className="font-medium text-fg">
              {activeServer ? activeServer.name : 'Rig Desktop'}
            </span>
            {activeServer && (
              <>
                <span>/</span>
                <span className="capitalize">
                  {section === 'local' ? 'Local Repos' : section}
                </span>
              </>
            )}
          </div>
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={() => setAppearanceOpen(true)}
              title="Customize Appearance"
              className="inline-flex items-center gap-1.5 rounded-lg border border-line bg-bg px-2.5 py-1 text-xs text-muted hover:border-emerald-500/40 hover:text-fg"
            >
              <Palette className="size-3.5" />
              <span className="hidden sm:inline">Appearance</span>
            </button>
            <button
              type="button"
              onClick={() => setPaletteOpen(true)}
              className="inline-flex items-center gap-2 rounded-lg border border-line bg-bg px-2.5 py-1 text-xs text-muted hover:border-emerald-500/40 hover:text-fg"
            >
              <Command className="size-3" />
              <span>Jump to…</span>
              <kbd className="rounded bg-surface-2 px-1 py-0.5 font-mono text-[10px]">Ctrl+K</kbd>
            </button>
          </div>
        </header>

        <main className="flex min-h-0 flex-1 flex-col overflow-y-auto p-5">
          {!activeServer ? (
            <div className="flex flex-1 flex-col items-center justify-center p-8 text-center">
              <div className="mb-4 flex size-14 items-center justify-center rounded-2xl bg-emerald-500/15 text-emerald-400 ring-1 ring-emerald-500/30">
                <Server className="size-7" />
              </div>
              <h1 className="text-xl font-semibold tracking-tight text-fg">
                Control all your Rig servers in one place
              </h1>
              <p className="mt-2 max-w-md text-xs leading-relaxed text-muted">
                Pair headless or full Rig servers over public HTTPS, LAN (pinned internal CA), or
                Tailscale. Tokens stay in your OS keychain and all streaming runs in the native Rust
                core.
              </p>
              <div className="mt-6 flex flex-wrap items-center justify-center gap-2.5">
                <button
                  type="button"
                  onClick={() => setAddOpen(true)}
                  className="inline-flex items-center gap-1.5 rounded-xl bg-emerald-600 px-4 py-2.5 text-xs font-semibold text-white hover:bg-emerald-500"
                >
                  <Plus className="size-4" />
                  Add Rig Server
                </button>
                <button
                  type="button"
                  onClick={() => setAppearanceOpen(true)}
                  className="inline-flex items-center gap-1.5 rounded-xl border border-line bg-surface px-4 py-2.5 text-xs font-medium text-fg hover:bg-surface-2"
                >
                  <Palette className="size-4 text-emerald-400" />
                  Customize Appearance
                </button>
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
                  onCreateDatabase={openCreateDatabase}
                />
              )}
              {section === 'apps' && (
                <AppsView
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
          setSection(prefs.defaultSection)
          void qc.invalidateQueries({ queryKey: ['desktop', 'servers'] })
        }}
      />

      <AppearanceModal open={appearanceOpen} onClose={() => setAppearanceOpen(false)} />

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
          setSection(sec)
          if (appId) setSelectedAppId(appId)
          if (tab) setAppTab(tab as AppDetailTab)
        }}
        onAddServer={() => setAddOpen(true)}
        onDeployApp={() => openDeployModal()}
        onCreateDatabase={openCreateDatabase}
        onOpenAppearance={() => setAppearanceOpen(true)}
      />
    </div>
  )
}
