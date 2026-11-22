import createClient, { type Middleware } from 'openapi-fetch'
import type { components, paths } from './schema'

export type Schemas = components['schemas']

/** An error returned by the Rig API, or a network failure (status 0). */
export class ApiError extends Error {
  readonly status: number
  readonly code: string
  readonly challengeToken?: string

  constructor(status: number, code: string, message: string, challengeToken?: string) {
    super(message)
    this.name = 'ApiError'
    this.status = status
    this.code = code
    this.challengeToken = challengeToken
  }
}

let csrfToken = ''

/** Called whenever the session is (re)loaded; writes need this token. */
export function setCsrfToken(token: string) {
  csrfToken = token
}

const SAFE_METHODS = new Set(['GET', 'HEAD', 'OPTIONS'])

const csrf: Middleware = {
  onRequest({ request }) {
    if (csrfToken && !SAFE_METHODS.has(request.method)) {
      request.headers.set('x-rig-csrf', csrfToken)
    }
    return request
  },
  onResponse({ response }) {
    const rotated = response.headers.get('x-rig-csrf')
    if (rotated) {
      csrfToken = rotated
    }
    return response
  },
}

export const api = createClient<paths>({ baseUrl: '/', credentials: 'same-origin' })
api.use(csrf)

type Result<T> = { data?: T; error?: unknown; response: Response }

function messageFor(status: number): string {
  if (status === 401) return 'Your session has ended. Sign in again.'
  if (status === 403) return 'You do not have permission to do that.'
  if (status === 404) return 'That no longer exists.'
  if (status === 429) return 'Too many attempts. Wait a few minutes and try again.'
  if (status >= 500) return 'The server hit a problem. Check the Rig logs.'
  return 'The request failed.'
}

/**
 * Resolves an openapi-fetch call to its data, or throws an `ApiError`.
 * Keeps every query and mutation function to one line.
 */
export async function unwrap<T>(call: Promise<Result<T>>): Promise<T> {
  let result: Result<T>
  try {
    result = await call
  } catch {
    throw new ApiError(0, 'network', 'Cannot reach the Rig server.')
  }
  const { data, error, response } = result
  if (response.ok) return data as T
  const detail = (error as Schemas['ErrorBody'] | undefined)?.error
  throw new ApiError(
    response.status,
    detail?.code ?? 'error',
    detail?.message ?? messageFor(response.status),
    detail?.challenge_token ?? undefined,
  )
}
