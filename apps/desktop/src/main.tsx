import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { Toaster } from 'sonner'
import App from './App'
import { PreferencesProvider, usePreferences } from './lib/preferences'
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
  return <Toaster position="bottom-right" theme={resolvedTheme} richColors closeButton />
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <PreferencesProvider>
        <App />
        <ThemedToaster />
      </PreferencesProvider>
    </QueryClientProvider>
  </StrictMode>,
)
