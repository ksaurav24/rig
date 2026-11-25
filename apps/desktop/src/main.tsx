import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { Toaster } from 'sonner'
import App from './App'
import { TooltipProvider } from '@rig/ui/tooltip'
import { FluidProvider } from '@rig/ui/lib/fluid-provider'
import { FluidPreferencesSync, PreferencesProvider, usePreferences } from './lib/preferences'
import './index.css'

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 10_000,
      retry: 1,
      refetchOnWindowFocus: false,
    },
  },
})

function ThemedToaster() {
  const { resolvedTheme } = usePreferences()
  return (
    <Toaster
      position="bottom-right"
      theme={resolvedTheme}
      richColors
      closeButton
      toastOptions={{
        style: {
          fontFamily: 'var(--font-sans)',
          fontSize: '13.5px',
        },
      }}
    />
  )
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <FluidProvider>
        <TooltipProvider>
          <PreferencesProvider>
            <FluidPreferencesSync />
            <App />
            <ThemedToaster />
          </PreferencesProvider>
        </TooltipProvider>
      </FluidProvider>
    </QueryClientProvider>
  </StrictMode>,
)
