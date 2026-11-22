import { QueryClientProvider } from '@tanstack/react-query'
import { RouterProvider, createRouter } from '@tanstack/react-router'
import { FluidProvider } from '@/lib/fluid-provider'
import { FluidPreferencesSync, PreferencesProvider, usePreferences } from '@/lib/preferences'
import { ThemeProvider } from '@/lib/theme'
import { StrictMode, type ReactNode } from 'react'
import { createRoot } from 'react-dom/client'
import { Toaster } from '@/components/ui/sonner'
import { TooltipProvider } from '@/components/fluid/tooltip'
import { queryClient, setSignedOutHandler } from '@/lib/query'
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

function FluidWithPreferences({ children }: { children: ReactNode }) {
  const { prefs } = usePreferences()
  return (
    <FluidProvider shape={prefs.shape} size={prefs.density}>
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
