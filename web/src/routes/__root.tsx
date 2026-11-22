import type { QueryClient } from '@tanstack/react-query'
import { Link, Outlet, createRootRouteWithContext } from '@tanstack/react-router'
import { Compass } from 'lucide-react'
import { EmptyState, ErrorState } from '@/components/app/state'
import { buttonVariants } from '@/components/ui/button'

export const Route = createRootRouteWithContext<{ queryClient: QueryClient }>()({
  component: Outlet,
  notFoundComponent: () => (
    <div className="mx-auto max-w-xl p-8">
      <EmptyState
        icon={Compass}
        title="Page not found"
        action={
          <Link to="/" className={buttonVariants({ variant: 'outline', size: 'sm' })}>
            Go to overview
          </Link>
        }
      >
        The address may be mistyped, or the resource was deleted.
      </EmptyState>
    </div>
  ),
  errorComponent: ({ error, reset }) => (
    <div className="mx-auto max-w-xl p-8">
      <ErrorState error={error} onRetry={reset} />
    </div>
  ),
})
