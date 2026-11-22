import { queryOptions } from '@tanstack/react-query'
import { api, setCsrfToken, unwrap } from './client'

export const setupStatusQuery = queryOptions({
  queryKey: ['setup'],
  queryFn: () => unwrap(api.GET('/api/v1/setup')),
  staleTime: 60_000,
})

export const meQuery = queryOptions({
  queryKey: ['me'],
  queryFn: async () => {
    const me = await unwrap(api.GET('/api/v1/auth/me'))
    setCsrfToken(me.csrf_token)
    return me
  },
  staleTime: 5 * 60_000,
})

export const alertChannelsQuery = queryOptions({
  queryKey: ['alert-channels'],
  queryFn: () => unwrap(api.GET('/api/v1/alerts/channels')),
})

export const tokensQuery = queryOptions({
  queryKey: ['tokens'],
  queryFn: () => unwrap(api.GET('/api/v1/tokens')),
})

export const membersQuery = queryOptions({
  queryKey: ['members'],
  queryFn: () => unwrap(api.GET('/api/v1/members')),
})

export const auditQuery = queryOptions({
  queryKey: ['audit'],
  queryFn: () => unwrap(api.GET('/api/v1/audit')),
})

export const systemQuery = queryOptions({
  queryKey: ['system'],
  queryFn: () => unwrap(api.GET('/api/v1/system')),
  refetchInterval: 10_000,
})

export const networkQuery = queryOptions({
  queryKey: ['system', 'network'],
  queryFn: () => unwrap(api.GET('/api/v1/system/network')),
  staleTime: 60_000,
})

export const projectsQuery = queryOptions({
  queryKey: ['projects'],
  queryFn: () => unwrap(api.GET('/api/v1/projects')),
})

/** Statuses that change on their own, so lists showing them poll. */
const isBusy = (status: string) => status === 'deploying' || status === 'queued' || status === 'building'

export const appsQuery = queryOptions({
  queryKey: ['apps'],
  queryFn: () => unwrap(api.GET('/api/v1/apps')),
  refetchInterval: (q) => (q.state.data?.some((a) => isBusy(a.status)) ? 2_000 : 30_000),
})

// Per-app keys all start with ['apps', id], so one invalidation refreshes
// everything shown for an app.
export const appQuery = (id: string) =>
  queryOptions({
    queryKey: ['apps', id],
    queryFn: () => unwrap(api.GET('/api/v1/apps/{id}', { params: { path: { id } } })),
    refetchInterval: (q) => (q.state.data && isBusy(q.state.data.status) ? 2_000 : 30_000),
  })

export const deploymentsQuery = (id: string) =>
  queryOptions({
    queryKey: ['apps', id, 'deployments'],
    queryFn: () => unwrap(api.GET('/api/v1/apps/{id}/deployments', { params: { path: { id } } })),
    refetchInterval: (q) => (q.state.data?.some((d) => isBusy(d.status)) ? 2_000 : false),
  })

export const envQuery = (id: string) =>
  queryOptions({
    queryKey: ['apps', id, 'env'],
    queryFn: () => unwrap(api.GET('/api/v1/apps/{id}/env', { params: { path: { id } } })),
  })

export const domainsQuery = (id: string) =>
  queryOptions({
    queryKey: ['apps', id, 'domains'],
    queryFn: () => unwrap(api.GET('/api/v1/apps/{id}/domains', { params: { path: { id } } })),
  })

export const databasesQuery = queryOptions({
  queryKey: ['databases'],
  queryFn: () => unwrap(api.GET('/api/v1/databases')),
  refetchInterval: (q) => (q.state.data?.some((d) => d.status === 'creating') ? 2_000 : 30_000),
})

export const backupsQuery = (databaseId: string) =>
  queryOptions({
    queryKey: ['databases', databaseId, 'backups'],
    queryFn: () => unwrap(api.GET('/api/v1/databases/{id}/backups', { params: { path: { id: databaseId } } })),
    refetchInterval: (q) => (q.state.data?.some((b) => b.status === 'running') ? 1_500 : false),
  })

// The offered engines only change with a Rig upgrade.
export const databaseEnginesQuery = queryOptions({
  queryKey: ['databases', 'engines'],
  queryFn: () => unwrap(api.GET('/api/v1/databases/engines')),
  staleTime: Infinity,
})

export type MetricTarget = { kind: 'host' } | { kind: 'app'; id: string } | { kind: 'database'; id: string }
export type MetricRange = '1h' | '6h' | '24h'

// The server samples every 30 seconds, so asking more often shows nothing new.
export const metricsQuery = (target: MetricTarget, range: MetricRange) =>
  queryOptions({
    // App series live under ['apps', id] so they refresh with the rest of the app.
    queryKey:
      target.kind === 'host'
        ? ['system', 'metrics', range]
        : [target.kind === 'app' ? 'apps' : 'databases', target.id, 'metrics', range],
    queryFn: () => {
      const query = { range }
      if (target.kind === 'host') return unwrap(api.GET('/api/v1/system/metrics', { params: { query } }))
      const params = { path: { id: target.id }, query }
      return target.kind === 'app'
        ? unwrap(api.GET('/api/v1/apps/{id}/metrics', { params }))
        : unwrap(api.GET('/api/v1/databases/{id}/metrics', { params }))
    },
    refetchInterval: 30_000,
    staleTime: 15_000,
    placeholderData: (previous) => previous,
  })

export const githubQuery = queryOptions({
  queryKey: ['github'],
  queryFn: () => unwrap(api.GET('/api/v1/github')),
})

export const githubReposQuery = queryOptions({
  queryKey: ['github', 'repos'],
  queryFn: () => unwrap(api.GET('/api/v1/github/repos')),
  staleTime: 60_000,
})

/** Branches of a repository the GitHub App can read, default branch first. */
export const githubBranchesQuery = (fullName: string) => {
  const [owner = '', repo = ''] = fullName.split('/')
  return queryOptions({
    queryKey: ['github', 'repos', fullName, 'branches'],
    queryFn: () => unwrap(api.GET('/api/v1/github/repos/{owner}/{repo}/branches', { params: { path: { owner, repo } } })),
    retry: 1,
    staleTime: 30_000,
  })
}
