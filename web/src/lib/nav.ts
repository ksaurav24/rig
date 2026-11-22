import { Bell, Boxes, Database, GitBranch, KeyRound, LayoutDashboard, LayoutGrid, Network, ScrollText, SlidersHorizontal, UserRound, Users, type LucideIcon } from 'lucide-react'
import type { FileRouteTypes } from '@/routeTree.gen'
import type { Schemas } from '@/api/client'

type Role = Schemas['Role']

export interface NavItem {
  title: string
  to: FileRouteTypes['to']
  icon: LucideIcon
  /** Lowest role that can open the page. Omitted means everyone. */
  minRole?: Role
  keywords?: string
}

export interface NavGroup {
  label: string
  items: NavItem[]
}

const RANK: Record<Role, number> = { viewer: 0, developer: 1, admin: 2, owner: 3 }

export const ROLES: { value: Role; label: string; hint: string }[] = [
  { value: 'viewer', label: 'Viewer', hint: 'Read apps, logs and metrics' },
  { value: 'developer', label: 'Developer', hint: 'Deploy and change apps' },
  { value: 'admin', label: 'Admin', hint: 'Manage members and settings' },
  { value: 'owner', label: 'Owner', hint: 'Everything, including managing admins and owners' },
]

export function hasRole(role: Role, min: Role | undefined): boolean {
  return !min || RANK[role] >= RANK[min]
}

/** Primary workspace routes shown at the top of the sidebar. */
export const WORKSPACE_NAV: NavGroup = {
  label: 'Workspace',
  items: [
    { title: 'Overview', to: '/', icon: LayoutDashboard, keywords: 'home dashboard' },
    { title: 'Projects', to: '/projects', icon: Boxes, keywords: 'apps services deploy' },
    { title: 'Databases', to: '/databases', icon: Database, keywords: 'postgres mysql mariadb redis mongodb mongo sql nosql' },
    { title: 'Catalog', to: '/catalog', icon: LayoutGrid, keywords: 'templates one-click marketplace install apps' },
  ],
}

/** Organization & server settings shown in the sidebar. */
export const SETTINGS_NAV: NavGroup = {
  label: 'Settings',
  items: [
    { title: 'General', to: '/settings/general', icon: SlidersHorizontal, minRole: 'admin', keywords: 'organization branding logo name accent environment workspace' },
    { title: 'Team', to: '/settings/team', icon: Users, minRole: 'admin', keywords: 'members users roles invite people' },
    { title: 'Network', to: '/settings/network', icon: Network, keywords: 'lan tailscale tailnet access firewall ip' },
    { title: 'GitHub', to: '/settings/github', icon: GitBranch, minRole: 'admin', keywords: 'git push deploy app repository webhook' },
    { title: 'Alerts', to: '/settings/alerts', icon: Bell, minRole: 'admin', keywords: 'notifications webhook slack discord' },
    { title: 'Audit log', to: '/settings/audit', icon: ScrollText, minRole: 'admin', keywords: 'activity history' },
  ],
}

/** Personal routes anchored in the sidebar user footer menu (and still searchable in ⌘K). */
export const PERSONAL_NAV: NavGroup = {
  label: 'Personal',
  items: [
    { title: 'Account', to: '/settings/account', icon: UserRound, keywords: 'password profile 2fa totp' },
    { title: 'Access tokens', to: '/settings/tokens', icon: KeyRound, keywords: 'api cli mcp key' },
  ],
}

/** Sidebar navigation groups: Workspace + org/server Settings (personal items live in the user menu). */
export function sidebarNavFor(role: Role): NavGroup[] {
  return [WORKSPACE_NAV, SETTINGS_NAV]
    .map((g) => ({ ...g, items: g.items.filter((i) => hasRole(role, i.minRole)) }))
    .filter((g) => g.items.length > 0)
}

/** All navigable routes for the command menu. */
export function navFor(role: Role): NavGroup[] {
  return [WORKSPACE_NAV, SETTINGS_NAV, PERSONAL_NAV]
    .map((g) => ({ ...g, items: g.items.filter((i) => hasRole(role, i.minRole)) }))
    .filter((g) => g.items.length > 0)
}
