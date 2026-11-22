import { useSuspenseQuery } from '@tanstack/react-query'
import { Link, Outlet, createFileRoute, redirect, useLocation, useMatches, useNavigate } from '@tanstack/react-router'
import { Fragment, useEffect, useState } from 'react'
import { ApiError } from '@/api/client'
import { meQuery, setupStatusQuery } from '@/api/queries'
import { AppSidebar } from '@/components/app/app-sidebar'
import { CommandMenu } from '@/components/app/command-menu'
import { DiskBanner } from '@/components/app/disk-banner'
import { ThemeToggle } from '@/components/app/theme-toggle'
import {
  Breadcrumb,
  BreadcrumbItem,
  BreadcrumbLink,
  BreadcrumbList,
  BreadcrumbPage,
} from '@/components/fluid/breadcrumb'
import {
  SIDEBAR_COOKIE_NAME,
  SidebarInset,
  SidebarProvider,
  SidebarTrigger,
} from '@/components/fluid/sidebar'
import { useSyncBranding } from '@/lib/branding'
import { usePreferences } from '@/lib/preferences'

declare module '@tanstack/react-router' {
  interface StaticDataRouteOption {
    /** Breadcrumb label for this route. */
    crumb?: string
  }
}

function readSidebarCookie(): boolean {
  if (typeof document === 'undefined') return true
  const match = document.cookie
    .split('; ')
    .find((entry) => entry.startsWith(`${SIDEBAR_COOKIE_NAME}=`))
  if (!match) return true
  return match.slice(SIDEBAR_COOKIE_NAME.length + 1) !== 'false'
}

export const Route = createFileRoute('/_app')({
  beforeLoad: async ({ context, location }) => {
    const status = await context.queryClient.ensureQueryData(setupStatusQuery)
    if (!status.setup_complete) throw redirect({ to: '/setup' })
    try {
      await context.queryClient.ensureQueryData(meQuery)
    } catch (e) {
      if (e instanceof ApiError && e.status === 401) {
        throw redirect({ to: '/login', search: location.pathname === '/' ? {} : { next: location.pathname } })
      }
      throw e
    }
  },
  component: AppLayout,
})

function AppLayout() {
  const { data: me } = useSuspenseQuery(meQuery)
  const { prefs } = usePreferences()
  const { pathname } = useLocation()
  const navigate = useNavigate()
  const [initialSidebarOpen] = useState(readSidebarCookie)

  useSyncBranding({
    orgName: me.org.name,
    logoUrl: me.org.logo_url,
    accentColor: me.org.accent_color,
  })

  // Apply preferred default home once per browser tab session when opening `/`,
  // while still letting the user click Overview in the sidebar anytime.
  useEffect(() => {
    try {
      if (sessionStorage.getItem('rig-home-applied')) return
      sessionStorage.setItem('rig-home-applied', '1')
    } catch {
      return
    }
    if (pathname === '/' && prefs.defaultHome !== '/') {
      void navigate({ to: prefs.defaultHome, replace: true })
    }
  }, [pathname, prefs.defaultHome, navigate])

  return (
    <SidebarProvider defaultOpen={initialSidebarOpen} persist peek={prefs.sidebarPeek}>
      <a
        href="#main"
        className="sr-only focus:not-sr-only focus:absolute focus:top-2 focus:left-2 focus:z-50 focus:rounded-md focus:bg-foreground focus:px-3 focus:py-1.5 focus:text-background"
      >
        Skip to content
      </a>
      <AppSidebar me={me} />
      <SidebarInset className="min-w-0 bg-background">
        <header className="sticky top-0 z-20 flex h-12 shrink-0 items-center gap-2 border-b border-border/75 bg-background/85 px-3 backdrop-blur-md">
          <SidebarTrigger />
          <Crumbs />
          <div className="ml-auto flex items-center gap-1.5">
            <CommandMenu me={me} />
            <ThemeToggle />
          </div>
        </header>
        <DiskBanner />
        <main id="main" className="min-w-0 flex-1">
          <Outlet />
        </main>
      </SidebarInset>
    </SidebarProvider>
  )
}

function Crumbs() {
  const crumbs = useMatches()
    .flatMap((m) => {
      // A loader can name its parents too, so a page outside a layout route
      // (an app, a project) still reads Projects › project › app.
      const data = m.loaderData as { crumb?: string; parents?: { label: string; to: string }[] } | undefined
      return [...(data?.parents ?? []), { label: data?.crumb ?? m.staticData.crumb, to: m.pathname }]
    })
    .filter((c): c is { label: string; to: string } => Boolean(c.label))
    // Nested matches can repeat a parent path; keep the deepest label.
    .filter((c, i, all) => all.findLastIndex((o) => o.to === c.to) === i)

  // Avoid repeating top-level page headings directly above their own <h1>.
  if (crumbs.length <= 1) return null

  return (
    <Breadcrumb size="compact" className="min-w-0">
      <BreadcrumbList className="flex-nowrap">
        {crumbs.map((c, i) => {
          const last = i === crumbs.length - 1
          return (
            <Fragment key={c.to}>
              <BreadcrumbItem className={last ? 'min-w-0' : 'hidden md:inline-flex'}>
                {last ? (
                  <BreadcrumbPage className="truncate">{c.label}</BreadcrumbPage>
                ) : (
                  <BreadcrumbLink render={<Link to={c.to} />}>{c.label}</BreadcrumbLink>
                )}
              </BreadcrumbItem>
            </Fragment>
          )
        })}
      </BreadcrumbList>
    </Breadcrumb>
  )
}
