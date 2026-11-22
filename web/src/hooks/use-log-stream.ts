import { useEffect, useState } from 'react'
import type { Schemas } from '@/api/client'

export type LogLine = Schemas['LogLine']
export type StreamState = 'connecting' | 'live' | 'ended' | 'error'

const MAX_LINES = 5000

/**
 * Follows a Rig log endpoint (server-sent events).
 *
 * Lines are buffered and flushed once per animation frame, so a chatty
 * build costs one render per frame rather than one per line.
 *
 * `replays`: the endpoint sends its history again on every connection (the
 * container log does; the deployment log resumes from `Last-Event-ID`).
 */
export function useLogStream(url: string | null, { replays = false }: { replays?: boolean } = {}) {
  const [lines, setLines] = useState<LogLine[]>([])
  const [state, setState] = useState<StreamState>('connecting')
  /** The `end` event's payload: a deployment's final status. */
  const [result, setResult] = useState<string | null>(null)

  useEffect(() => {
    setLines([])
    setResult(null)
    setState('connecting')
    if (!url) return

    const source = new EventSource(url)
    let pending: LogLine[] = []
    let frame = 0
    let dropHistory = false

    const flush = () => {
      frame = 0
      const batch = pending
      pending = []
      const reset = dropHistory
      dropHistory = false
      setLines((prev) => {
        const next = reset ? batch : prev.concat(batch)
        return next.length > MAX_LINES ? next.slice(next.length - MAX_LINES) : next
      })
    }

    source.addEventListener('open', () => {
      if (replays) {
        pending = []
        dropHistory = true
      }
      setState('live')
    })
    source.addEventListener('log', (e) => {
      try {
        pending.push(JSON.parse((e as MessageEvent<string>).data) as LogLine)
      } catch {
        return
      }
      frame ||= requestAnimationFrame(flush)
    })
    source.addEventListener('end', (e) => {
      setResult((e as MessageEvent<string>).data)
      setState('ended')
      source.close()
    })
    source.addEventListener('error', () => {
      // CLOSED means the server refused (not found, not running); otherwise
      // the browser is already reconnecting.
      setState(source.readyState === EventSource.CLOSED ? 'error' : 'connecting')
    })

    return () => {
      source.close()
      if (frame) cancelAnimationFrame(frame)
    }
  }, [url, replays])

  return { lines, state, result }
}
