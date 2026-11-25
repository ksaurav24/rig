import { FluidProvider } from '@rig/ui/lib/fluid-provider'
import type { LinkComponentProps } from '@rig/ui/lib/link-context'
import { TooltipProvider } from '@rig/ui/tooltip'
import { QueryClientProvider } from '@tanstack/react-query'
import { RouterProvider, createRouter } from '@tanstack/react-router'
import { StrictMode, type ReactNode } from 'react'
import { createRoot } from 'react-dom/client'
import { Toaster } from '@/components/ui/sonner'
import { FluidPreferencesSync, PreferencesProvider, usePreferences } from '@/lib/preferences'
import { queryClient, setSignedOutHandler } from '@/lib/query'
import { ThemeProvider } from '@/lib/theme'
import { routeTree } from './routeTree.gen'
import './index.css'

const router = createRouter({
  routeTree,
  context: { queryClient },
  defaultPreload: 'intent',
  // Query owns caching; the router should always ask it.
  defaultPreloadStaleTime: 0,
  scrollRestoration: true,
})

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router
  }
}

setSignedOutHandler(() => {
  const { pathname } = router.state.location
  if (pathname === '/login' || pathname === '/setup') return
  queryClient.removeQueries({ queryKey: ['me'] })
  void router.navigate({ to: '/login', search: { next: pathname } })
})

/** Bridges `@rig/ui` clickable links (e.g. Card) onto the TanStack Router instance. */
function RouterLink({ href, onClick, target, ...rest }: LinkComponentProps) {
  return (
    <a
      href={href}
      target={target}
      onClick={(e) => {
        onClick?.(e)
        if (e.defaultPrevented || target || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return
        if (/^[a-z][a-z0-9+.-]*:/i.test(href)) return
        e.preventDefault()
        void router.navigate({ to: href })
      }}
      {...rest}
    />
  )
}

function FluidWithPreferences({ children }: { children: ReactNode }) {
  const { prefs } = usePreferences()
  return (
    <FluidProvider shape={prefs.shape} size={prefs.density} linkComponent={RouterLink}>
      <FluidPreferencesSync />
      {children}
    </FluidProvider>
  )
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <ThemeProvider>
      <PreferencesProvider>
        <FluidWithPreferences>
          <QueryClientProvider client={queryClient}>
            <TooltipProvider delayDuration={300}>
              <RouterProvider router={router} />
              <Toaster position="bottom-right" closeButton />
            </TooltipProvider>
          </QueryClientProvider>
        </FluidWithPreferences>
      </PreferencesProvider>
    </ThemeProvider>
  </StrictMode>,
)
