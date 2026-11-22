import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Link, useLocation, useNavigate } from '@tanstack/react-router'
import { BookOpen, Boxes, ChevronsUpDown, LogOut, PinOff } from 'lucide-react'
import { useMemo } from 'react'
import { api, unwrap, type Schemas } from '@/api/client'
import { projectsQuery } from '@/api/queries'
import { OrgMark } from '@/components/app/logo'
import { UserAvatar } from '@/components/app/user-avatar'
import { Badge } from '@/components/fluid/badge'
import {
  DropdownContent,
  DropdownLabel,
  DropdownMenu,
  DropdownSeparator,
  DropdownTrigger,
  MenuItem,
} from '@/components/fluid/dropdown'
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuAction,
  SidebarMenuButton,
  SidebarMenuItem,
} from '@/components/fluid/sidebar'
import { PERSONAL_NAV, sidebarNavFor } from '@/lib/nav'
import { isEnvBadgeColor, usePreferences } from '@/lib/preferences'

function isNavActive(pathname: string, to: string): boolean {
  if (to === '/') return pathname === '/'
  return pathname === to || pathname.startsWith(`${to}/`)
}

export function AppSidebar({ me }: { me: Schemas['Me'] }) {
  const { pathname } = useLocation()
  const { prefs, togglePinnedProject } = usePreferences()
  const projects = useQuery(projectsQuery)
  const groups = sidebarNavFor(me.role)

  const pinned = useMemo(() => {
    if (prefs.pinnedProjects.length === 0 || !projects.data) return []
    const byId = new Map(projects.data.map((p) => [p.id, p]))
    return prefs.pinnedProjects
      .map((id) => byId.get(id))
      .filter((p): p is Schemas['Project'] => Boolean(p))
  }, [prefs.pinnedProjects, projects.data])

  const envColor = isEnvBadgeColor(me.org.env_color) ? me.org.env_color : 'emerald'

  return (
    <Sidebar variant={prefs.sidebarVariant} collapsible="offcanvas">
      <SidebarHeader className="px-2.5 pt-3 pb-1">
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton size="lg" render={<Link to="/" />}>
              <OrgMark logoUrl={me.org.logo_url} name={me.org.name} className="size-7 shrink-0 rounded-lg" />
              <span className="flex min-w-0 flex-1 items-center gap-2 leading-tight">
                <span className="truncate font-semibold text-foreground">{me.org.name}</span>
                {me.org.env_label ? (
                  <Badge variant="solid" color={envColor} size="compact" className="shrink-0">
                    {me.org.env_label}
                  </Badge>
                ) : null}
              </span>
            </SidebarMenuButton>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarHeader>

      <SidebarContent className="px-2 py-2">
        {groups.map((group, idx) => {
          const isSettings = group.label === 'Settings'
          return (
            <div key={group.label}>
              {idx === 1 && pinned.length > 0 ? (
                <SidebarGroup collapsible defaultOpen>
                  <SidebarGroupLabel>Pinned</SidebarGroupLabel>
                  <SidebarGroupContent>
                    <SidebarMenu>
                      {pinned.map((project) => {
                        const to = `/projects/${project.id}`
                        const active = isNavActive(pathname, to)
                        return (
                          <SidebarMenuItem key={project.id}>
                            <SidebarMenuButton
                              isActive={active}
                              icon={Boxes}
                              render={
                                <Link
                                  to="/projects/$projectId"
                                  params={{ projectId: project.id }}
                                  aria-current={active ? 'page' : undefined}
                                />
                              }
                            >
                              {project.name}
                            </SidebarMenuButton>
                            <SidebarMenuAction
                              showOnHover
                              aria-label={`Unpin ${project.name}`}
                              title="Unpin project"
                              onClick={() => togglePinnedProject(project.id)}
                            >
                              <PinOff className="size-3.5" />
                            </SidebarMenuAction>
                          </SidebarMenuItem>
                        )
                      })}
                    </SidebarMenu>
                  </SidebarGroupContent>
                </SidebarGroup>
              ) : null}

              <SidebarGroup collapsible={isSettings} defaultOpen>
                <SidebarGroupLabel>{group.label}</SidebarGroupLabel>
                <SidebarGroupContent>
                  <SidebarMenu>
                    {group.items.map((item) => {
                      const active = isNavActive(pathname, item.to)
                      return (
                        <SidebarMenuItem key={item.to}>
                          <SidebarMenuButton
                            isActive={active}
                            icon={item.icon}
                            render={
                              <Link
                                to={item.to}
                                aria-current={active ? 'page' : undefined}
                              />
                            }
                          >
                            {item.title}
                          </SidebarMenuButton>
                        </SidebarMenuItem>
                      )
                    })}
                  </SidebarMenu>
                </SidebarGroupContent>
              </SidebarGroup>
            </div>
          )
        })}
      </SidebarContent>

      <SidebarFooter className="border-t border-sidebar-border/70 px-2 py-2">
        <UserMenu me={me} />
      </SidebarFooter>
    </Sidebar>
  )
}

function UserMenu({ me }: { me: Schemas['Me'] }) {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const signOut = useMutation({
    mutationFn: () => unwrap(api.POST('/api/v1/auth/logout')),
    onSettled: async () => {
      queryClient.clear()
      await navigate({ to: '/login' })
    },
  })

  return (
    <SidebarMenu>
      <SidebarMenuItem>
        <DropdownMenu>
          <DropdownTrigger
            render={
              <SidebarMenuButton size="lg" aria-label="Account menu" className="w-full" />
            }
          >
            <UserAvatar
              name={me.user.name}
              avatarUrl={me.user.avatar_url}
              className="size-7 rounded-md"
            />
            <span className="grid min-w-0 flex-1 text-left leading-tight">
              <span className="truncate text-[13px] font-medium text-foreground">{me.user.name}</span>
              <span className="truncate text-[11px] text-muted-foreground capitalize">{me.role}</span>
            </span>
            <ChevronsUpDown className="ml-auto size-3.5 shrink-0 text-muted-foreground" />
          </DropdownTrigger>
          <DropdownContent side="top" align="start" className="w-60">
            <DropdownLabel className="py-1.5">
              <span className="block truncate text-[13px] font-medium text-foreground">{me.user.name}</span>
              <span className="block truncate text-[11.5px] font-normal text-muted-foreground">{me.user.email}</span>
            </DropdownLabel>
            <DropdownSeparator />
            {PERSONAL_NAV.items.map((item, idx) => (
              <MenuItem
                key={item.to}
                index={idx}
                icon={item.icon}
                label={item.title}
                onSelect={() => void navigate({ to: item.to })}
              />
            ))}
            <DropdownSeparator />
            <MenuItem
              index={PERSONAL_NAV.items.length}
              icon={BookOpen}
              label="API reference"
              onSelect={() => window.open('/api/docs', '_blank', 'noreferrer')}
            />
            <DropdownSeparator />
            <MenuItem
              index={PERSONAL_NAV.items.length + 1}
              icon={LogOut}
              label="Sign out"
              destructive
              disabled={signOut.isPending}
              onSelect={() => signOut.mutate()}
            />
          </DropdownContent>
        </DropdownMenu>
      </SidebarMenuItem>
    </SidebarMenu>
  )
}
