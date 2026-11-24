import {
  Box,
  Database,
  FolderGit2,
  LayoutDashboard,
  Palette,
  Plus,
  Rocket,
  Search,
  Server,
  Settings,
  Terminal,
} from 'lucide-react'
import { useEffect, useMemo, useState, type ReactNode } from 'react'
import type { App, Database as DbType, ServerView } from '@/lib/types'

export interface PaletteAction {
  id: string
  group: string
  label: string
  hint?: string
  icon: ReactNode
  run: () => void
}

interface CommandPaletteProps {
  open: boolean
  onClose: () => void
  servers: ServerView[]
  activeServer: ServerView | null
  apps: App[]
  databases: DbType[]
  onSelectServer: (id: string) => void
  onNavigate: (
    section: 'overview' | 'apps' | 'local' | 'databases' | 'settings',
    appId?: string,
    appTab?: string,
  ) => void
  onAddServer: () => void
  onDeployApp: () => void
  onCreateDatabase: () => void
  onOpenAppearance: () => void
}

export function CommandPalette({
  open,
  onClose,
  servers,
  activeServer,
  apps,
  databases,
  onSelectServer,
  onNavigate,
  onAddServer,
  onDeployApp,
  onCreateDatabase,
  onOpenAppearance,
}: CommandPaletteProps) {
  const [query, setQuery] = useState('')
  const [selected, setSelected] = useState(0)

  const items = useMemo<PaletteAction[]>(() => {
    const out: PaletteAction[] = []

    if (activeServer) {
      out.push(
        {
          id: 'act-deploy-app',
          group: 'Quick Actions',
          label: 'Deploy New Application…',
          hint: 'Local repo, Docker image, or Git URL',
          icon: <Rocket className="size-3.5 text-emerald-400" />,
          run: onDeployApp,
        },
        {
          id: 'act-create-db',
          group: 'Quick Actions',
          label: 'Create Managed Database…',
          hint: 'PostgreSQL, Redis, MySQL, MongoDB',
          icon: <Database className="size-3.5 text-sky-400" />,
          run: onCreateDatabase,
        },
        {
          id: 'nav-local',
          group: activeServer.name,
          label: 'Local Repositories',
          hint: 'Auto-discovered projects on this laptop',
          icon: <FolderGit2 className="size-3.5 text-emerald-400" />,
          run: () => onNavigate('local'),
        },
        {
          id: 'nav-overview',
          group: activeServer.name,
          label: 'Overview',
          hint: 'Host metrics & events',
          icon: <LayoutDashboard className="size-3.5" />,
          run: () => onNavigate('overview'),
        },
        {
          id: 'nav-apps',
          group: activeServer.name,
          label: 'Applications',
          hint: `${apps.length} apps`,
          icon: <Box className="size-3.5" />,
          run: () => onNavigate('apps'),
        },
        {
          id: 'nav-dbs',
          group: activeServer.name,
          label: 'Databases',
          hint: `${databases.length} databases`,
          icon: <Database className="size-3.5" />,
          run: () => onNavigate('databases'),
        },
        {
          id: 'nav-settings',
          group: activeServer.name,
          label: 'Server Settings',
          hint: 'Org, Appearance, Team, Devices, Alerts, Audit Log',
          icon: <Settings className="size-3.5" />,
          run: () => onNavigate('settings'),
        },
      )

      for (const app of apps) {
        out.push({
          id: `app-${app.id}`,
          group: 'Apps',
          label: app.name,
          hint: app.status,
          icon: <Box className="size-3.5 text-emerald-400" />,
          run: () => onNavigate('apps', app.id, 'deployments'),
        })
        if (app.status === 'running') {
          out.push({
            id: `term-${app.id}`,
            group: 'Terminals',
            label: `Terminal: ${app.name}`,
            hint: 'Open container shell',
            icon: <Terminal className="size-3.5 text-sky-400" />,
            run: () => onNavigate('apps', app.id, 'terminal'),
          })
        }
      }
    }

    for (const s of servers) {
      out.push({
        id: `srv-${s.id}`,
        group: 'Switch Server',
        label: s.name,
        hint: s.url,
        icon: <Server className="size-3.5" />,
        run: () => onSelectServer(s.id),
      })
    }

    out.push(
      {
        id: 'act-add-server',
        group: 'Actions',
        label: 'Add Rig Server…',
        hint: 'Pair code, password, or initialize',
        icon: <Plus className="size-3.5 text-emerald-400" />,
        run: onAddServer,
      },
      {
        id: 'act-appearance',
        group: 'Preferences',
        label: 'Customize Appearance & UI…',
        hint: 'Theme, surface, accent, density, corners, logs',
        icon: <Palette className="size-3.5 text-emerald-400" />,
        run: onOpenAppearance,
      },
    )

    const q = query.trim().toLowerCase()
    if (!q) return out
    return out.filter(
      (i) =>
        i.label.toLowerCase().includes(q) ||
        i.group.toLowerCase().includes(q) ||
        (i.hint && i.hint.toLowerCase().includes(q)),
    )
  }, [
    activeServer,
    apps,
    databases,
    onAddServer,
    onCreateDatabase,
    onDeployApp,
    onNavigate,
    onOpenAppearance,
    onSelectServer,
    query,
    servers,
  ])

  useEffect(() => {
    setSelected(0)
  }, [query, open])

  useEffect(() => {
    if (!open) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        onClose()
      } else if (e.key === 'ArrowDown') {
        e.preventDefault()
        setSelected((s) => Math.min(items.length - 1, s + 1))
      } else if (e.key === 'ArrowUp') {
        e.preventDefault()
        setSelected((s) => Math.max(0, s - 1))
      } else if (e.key === 'Enter' && items[selected]) {
        e.preventDefault()
        items[selected].run()
        onClose()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [items, onClose, open, selected])

  if (!open) return null

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/65 pt-[14vh] p-4"
      onClick={onClose}
    >
      <div
        className="w-full max-w-lg overflow-hidden rounded-2xl border border-line bg-surface shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center gap-2.5 border-b border-line px-4 py-3">
          <Search className="size-4 text-muted" />
          <input
            type="text"
            autoFocus
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={
              activeServer
                ? `Search ${activeServer.name} apps, deploy, databases, servers…`
                : 'Search servers and actions…'
            }
            className="flex-1 bg-transparent text-sm text-fg outline-none placeholder:text-muted"
          />
          <kbd className="rounded bg-surface-2 px-1.5 py-0.5 font-mono text-[10px] text-muted ring-1 ring-line">
            ESC
          </kbd>
        </div>
        <div className="max-h-80 overflow-y-auto p-1.5">
          {items.length === 0 ? (
            <div className="py-8 text-center text-xs text-muted">No matching commands</div>
          ) : (
            items.map((item, idx) => (
              <button
                key={item.id}
                type="button"
                onMouseEnter={() => setSelected(idx)}
                onClick={() => {
                  item.run()
                  onClose()
                }}
                className={`flex w-full items-center gap-2.5 rounded-lg px-3 py-2 text-left text-xs ${
                  idx === selected ? 'bg-surface-2 text-fg' : 'text-fg/80 hover:bg-surface-2/60'
                }`}
              >
                <span className="shrink-0 text-muted">{item.icon}</span>
                <span className="min-w-0 flex-1 truncate font-medium">{item.label}</span>
                {item.hint && (
                  <span className="max-w-[180px] truncate font-mono text-[10px] text-muted">
                    {item.hint}
                  </span>
                )}
                <span className="rounded bg-bg px-1.5 py-0.5 text-[10px] text-muted">
                  {item.group}
                </span>
              </button>
            ))
          )}
        </div>
      </div>
    </div>
  )
}
