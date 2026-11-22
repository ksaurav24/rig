import { MutationCache, QueryCache, QueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'
import { ApiError } from '@/api/client'

declare module '@tanstack/react-query' {
  interface Register {
    mutationMeta: {
      /** Toast shown when the mutation succeeds. */
      success?: string
      /** Set to handle the error in the component instead of a toast. */
      quietError?: boolean
    }
  }
}

function describe(error: unknown): string {
  return error instanceof Error ? error.message : 'Something went wrong.'
}

/** Fired once when any request finds the session gone. Set by the router. */
let onSignedOut: () => void = () => {}
export function setSignedOutHandler(handler: () => void) {
  onSignedOut = handler
}

function handleAuth(error: unknown): boolean {
  if (error instanceof ApiError && error.status === 401) {
    onSignedOut()
    return true
  }
  return false
}

export const queryClient = new QueryClient({
  // Notifications are wired here once, so call sites only declare intent
  // (`meta: { success: 'Token created' }`) and never import the toaster.
  mutationCache: new MutationCache({
    onSuccess(_data, _vars, _ctx, mutation) {
      const message = mutation.meta?.success
      if (message) toast.success(message)
    },
    onError(error, _vars, _ctx, mutation) {
      if (handleAuth(error) || mutation.meta?.quietError) return
      toast.error(describe(error))
    },
  }),
  queryCache: new QueryCache({
    onError(error, query) {
      if (handleAuth(error)) return
      // Only interrupt for background refetch failures; a first load that
      // fails renders its own error state in place.
      if (query.state.data !== undefined) {
        toast.error(describe(error), { id: `query-${query.queryHash}` })
      }
    },
  }),
  defaultOptions: {
    queries: {
      staleTime: 15_000,
      gcTime: 5 * 60_000,
      refetchOnWindowFocus: true,
      retry(failureCount, error) {
        if (error instanceof ApiError && error.status > 0 && error.status < 500) return false
        return failureCount < 2
      },
    },
    mutations: { retry: false },
  },
})
