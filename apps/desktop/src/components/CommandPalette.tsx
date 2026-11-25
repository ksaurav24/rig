import {
  Box,
  Database,
  FolderGit2,
  HardDriveDownload,
  Info,
  LayoutDashboard,
  LayoutGrid,
  Moon,
  Palette,
  Plus,
  Rocket,
  RotateCcw,
  Server,
  Settings,
  ShieldCheck,
  Sliders,
  Sun,
  Terminal,
  ZoomIn,
  ZoomOut,
} from 'lucide-react'
import { useMemo } from 'react'
import {
  CommandMenu,
  CommandMenuDialog,
  CommandMenuFooter,
  CommandMenuInput,
  CommandMenuList,
  type CommandMenuItemData,
} from '@rig/ui/command-menu'
import type { DesktopSettingsTab } from './DesktopSettingsView'
import { usePreferences } from '@/lib/preferences'
import type { App, Database as DbType, ServerView } from '@/lib/types'

interface CommandPaletteProps {
  open: boolean
  onClose: () => void
  servers: ServerView[]
  activeServer: ServerView | null
  apps: App[]
  databases: DbType[]
  onSelectServer: (id: string) => void
  onNavigate: (
    section: 'overview' | 'apps' | 'catalog' | 'local' | 'databases' | 'settings',
    appId?: string,
    tab?: string,
  ) => void
  onAddServer: () => void
  onDeployApp: () => void
  onCreateDatabase: () => void
  onOpenAppSettings: (tab?: DesktopSettingsTab) => void
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
  onOpenAppSettings,
}: CommandPaletteProps) {
  const { prefs, resolvedTheme, updatePrefs, zoomIn, zoomOut, resetZoom } = usePreferences()

  const items = useMemo<CommandMenuItemData[]>(() => {
    const out: CommandMenuItemData[] = []

    if (activeServer) {
      out.push(
        {
          value: 'act-deploy-app',
          group: 'Quick Actions',
          label: 'Deploy New Application…',
          description: 'GitHub repo, local folder, Docker image, or Git URL',
          icon: Rocket,
          keywords: ['deploy', 'create', 'app', 'new', 'github'],
          onSelect: onDeployApp,
        },
        {
          value: 'act-create-db',
          group: 'Quick Actions',
          label: 'Create Managed Database…',
          description: 'PostgreSQL, MySQL, MariaDB, Redis, MongoDB',
          icon: Database,
          keywords: ['database', 'postgres', 'redis', 'mysql', 'mariadb', 'mongo'],
          onSelect: onCreateDatabase,
        },
        {
          value: 'nav-overview',
          group: activeServer.name,
          label: 'Overview',
          description: 'Host metrics & live events',
          icon: LayoutDashboard,
          onSelect: () => onNavigate('overview'),
        },
        {
          value: 'nav-apps',
          group: activeServer.name,
          label: 'Applications',
          description: `${apps.length} apps`,
          icon: Box,
          onSelect: () => onNavigate('apps'),
        },
        {
          value: 'nav-catalog',
          group: activeServer.name,
          label: 'App Catalog (One-Click Templates)',
          description: 'Uptime Kuma, Vaultwarden, n8n, Gitea, Jellyfin & more',
          icon: LayoutGrid,
          keywords: ['catalog', 'templates', 'store', 'self-hosted', 'docker'],
          onSelect: () => onNavigate('catalog'),
        },
        {
          value: 'nav-local',
          group: activeServer.name,
          label: 'Local Repositories',
          description: 'Auto-discovered projects on this machine',
          icon: FolderGit2,
          onSelect: () => onNavigate('local'),
        },
        {
          value: 'nav-dbs',
          group: activeServer.name,
          label: 'Databases',
          description: `${databases.length} managed databases`,
          icon: Database,
          onSelect: () => onNavigate('databases'),
        },
        {
          value: 'nav-backup',
          group: activeServer.name,
          label: 'Server Snapshot Backup & Restore',
          description: 'Download full snapshot, schedule reminders, or restore to any server',
          icon: HardDriveDownload,
          keywords: ['snapshot', 'backup', 'restore', 'migrate', 'reminder', 'export'],
          onSelect: () => onNavigate('settings', undefined, 'backup'),
        },
        {
          value: 'nav-account',
          group: activeServer.name,
          label: 'Account, Password & 2FA',
          description: 'Profile, avatar, password, and authenticator TOTP',
          icon: ShieldCheck,
          keywords: ['account', 'profile', 'avatar', 'password', '2fa', 'totp'],
          onSelect: () => onNavigate('settings', undefined, 'account'),
        },
        {
          value: 'nav-settings',
          group: activeServer.name,
          label: 'Server Settings',
          description: 'Org, Logo, Network, GitHub, Team, Devices, Alerts, Audit Log',
          icon: Settings,
          onSelect: () => onNavigate('settings', undefined, 'general'),
        },
      )

      for (const app of apps) {
        out.push({
          value: `app-${app.id}`,
          group: 'Applications',
          label: app.name,
          description: `${app.project_name} · ${app.status}`,
          icon: Box,
          keywords: [app.project_name, app.status, app.source_kind],
          onSelect: () => onNavigate('apps', app.id, 'deployments'),
        })
        if (app.status === 'running') {
          out.push({
            value: `term-${app.id}`,
            group: 'Terminals',
            label: `Terminal: ${app.name}`,
            description: 'Open interactive container shell',
            icon: Terminal,
            keywords: ['shell', 'exec', 'ssh', app.name],
            onSelect: () => onNavigate('apps', app.id, 'terminal'),
          })
        }
      }
    }

    servers.forEach((s, idx) => {
      out.push({
        value: `srv-${s.id}`,
        group: 'Switch Server',
        label: s.name,
        description: s.url,
        icon: Server,
        shortcut: idx < 9 ? `mod+${idx + 1}` : undefined,
        onSelect: () => onSelectServer(s.id),
      })
    })

    out.push(
      {
        value: 'act-add-server',
        group: 'Servers',
        label: 'Add Rig Server…',
        description: 'Pair code, password, or initialize',
        icon: Plus,
        shortcut: 'mod+n',
        onSelect: onAddServer,
      },
      {
        value: 'act-zoom-in',
        group: 'Interface',
        label: 'Zoom In',
        description: `Currently ${Math.round(prefs.zoom * 100)}%`,
        icon: ZoomIn,
        shortcut: 'mod++',
        keywords: ['scale', 'larger', 'bigger', 'font', 'size'],
        onSelect: zoomIn,
      },
      {
        value: 'act-zoom-out',
        group: 'Interface',
        label: 'Zoom Out',
        description: `Currently ${Math.round(prefs.zoom * 100)}%`,
        icon: ZoomOut,
        shortcut: 'mod+-',
        keywords: ['scale', 'smaller', 'font', 'size'],
        onSelect: zoomOut,
      },
      {
        value: 'act-zoom-reset',
        group: 'Interface',
        label: 'Reset Zoom to 100%',
        icon: RotateCcw,
        shortcut: 'mod+0',
        keywords: ['scale', 'default', 'reset'],
        onSelect: resetZoom,
      },
      {
        value: 'act-theme',
        group: 'Interface',
        label: resolvedTheme === 'dark' ? 'Switch to Light Mode' : 'Switch to Dark Mode',
        icon: resolvedTheme === 'dark' ? Sun : Moon,
        keywords: ['theme', 'dark', 'light'],
        onSelect: () => updatePrefs({ theme: resolvedTheme === 'dark' ? 'light' : 'dark' }),
      },
      {
        value: 'act-app-settings',
        group: 'App Settings',
        label: 'App Settings (General)',
        description: 'Landing view, sidebar, logs, timestamps, notifications',
        icon: Sliders,
        shortcut: 'mod+,',
        keywords: ['preferences', 'settings', 'general', 'desktop'],
        onSelect: () => onOpenAppSettings('general'),
      },
      {
        value: 'act-appearance',
        group: 'App Settings',
        label: 'Appearance & Zoom',
        description: 'Theme, surface, accent, zoom, density, corners',
        icon: Palette,
        keywords: ['theme', 'color', 'zoom', 'appearance'],
        onSelect: () => onOpenAppSettings('appearance'),
      },
      {
        value: 'act-manage-servers',
        group: 'App Settings',
        label: 'Manage Paired Servers',
        description: 'Rename, reorder, accent colors, or disconnect servers',
        icon: Server,
        keywords: ['servers', 'manage', 'rename', 'reorder'],
        onSelect: () => onOpenAppSettings('servers'),
      },
      {
        value: 'act-cli',
        group: 'App Settings',
        label: 'CLI & MCP Setup',
        description: 'Detect rig CLI binary, config, and MCP JSON snippet',
        icon: Terminal,
        keywords: ['cli', 'mcp', 'command line'],
        onSelect: () => onOpenAppSettings('cli'),
      },
      {
        value: 'act-updates',
        group: 'App Settings',
        label: 'Versions & Updates',
        description: 'Check for updates and view server version matrix',
        icon: Info,
        keywords: ['version', 'update', 'diagnostics', 'about'],
        onSelect: () => onOpenAppSettings('updates'),
      },
    )

    return out
  }, [
    activeServer,
    apps,
    databases.length,
    onAddServer,
    onCreateDatabase,
    onDeployApp,
    onNavigate,
    onOpenAppSettings,
    onSelectServer,
    prefs.zoom,
    resetZoom,
    resolvedTheme,
    servers,
    updatePrefs,
    zoomIn,
    zoomOut,
  ])

  return (
    <CommandMenuDialog
      open={open}
      onOpenChange={(next) => {
        if (!next) onClose()
      }}
      shortcut={null}
      title="Command Menu"
      description="Jump to servers, apps, databases, terminals, or adjust UI settings"
    >
      <CommandMenu items={items}>
        <CommandMenuInput
          placeholder={
            activeServer
              ? `Search ${activeServer.name} apps, deploy, zoom, servers…`
              : 'Search servers and actions…'
          }
        />
        <CommandMenuList />
        <CommandMenuFooter />
      </CommandMenu>
    </CommandMenuDialog>
  )
}
