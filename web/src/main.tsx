import { QueryClientProvider } from '@tanstack/react-query'
import { RouterProvider, createRouter } from '@tanstack/react-router'
import { FluidProvider } from '@/lib/fluid-provider'
import { ThemeProvider } from '@/lib/theme'
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { Toaster } from '@/components/ui/sonner'
import { TooltipProvider } from '@/components/ui/tooltip'
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

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <ThemeProvider>
      <FluidProvider>
      <QueryClientProvider client={queryClient}>
        <TooltipProvider delay={300}>
          <RouterProvider router={router} />
          <Toaster position="bottom-right" closeButton />
        </TooltipProvider>
      </QueryClientProvider>
      </FluidProvider>
    </ThemeProvider>
  </StrictMode>,
)
