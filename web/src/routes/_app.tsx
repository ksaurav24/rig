import { useSuspenseQuery } from '@tanstack/react-query'
import { Link, Outlet, createFileRoute, redirect, useMatches } from '@tanstack/react-router'
import { Fragment } from 'react'
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
  BreadcrumbSeparator,
} from '@/components/ui/breadcrumb'
import { Separator } from '@/components/ui/separator'
import { SidebarInset, SidebarProvider, SidebarTrigger } from '@/components/ui/sidebar'

declare module '@tanstack/react-router' {
  interface StaticDataRouteOption {
    /** Breadcrumb label for this route. */
    crumb?: string
  }
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
  return (
    <SidebarProvider>
      <a
        href="#main"
        className="sr-only focus:not-sr-only focus:absolute focus:top-2 focus:left-2 focus:z-50 focus:rounded-md focus:bg-primary focus:px-3 focus:py-1.5 focus:text-primary-foreground"
      >
        Skip to content
      </a>
      <AppSidebar me={me} />
      <SidebarInset className="min-w-0">
        <header className="sticky top-0 z-20 flex h-12 shrink-0 items-center gap-2 border-b bg-background/85 px-3 backdrop-blur">
          <SidebarTrigger />
          <Separator orientation="vertical" className="mr-1 h-4! self-center!" />
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

  return (
    <Breadcrumb className="min-w-0">
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
              {last ? null : <BreadcrumbSeparator className="hidden md:block" />}
            </Fragment>
          )
        })}
      </BreadcrumbList>
    </Breadcrumb>
  )
}
