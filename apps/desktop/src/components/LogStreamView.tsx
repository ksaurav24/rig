import { useVirtualizer } from '@tanstack/react-virtual'
import { ArrowDownToLine, Copy, Pause, Play, Search, WrapText, X } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { toast } from 'sonner'
import { formatLogTime } from '@/lib/format'
import { desktopApi } from '@/lib/ipc'
import { usePreferences } from '@/lib/preferences'
import type { AnsiSpan, LogChannelFrame, ParsedLogLine } from '@/lib/types'

const ANSI_COLORS: Record<number, string> = {
  0: '#71717a',
  1: '#f87171',
  2: '#34d399',
  3: '#fbbf24',
  4: '#60a5fa',
  5: '#c084fc',
  6: '#22d3ee',
  7: '#e4e4e7',
  8: '#a1a1aa',
  9: '#fca5a5',
  10: '#6ee7b7',
  11: '#fde68a',
  12: '#93c5fd',
  13: '#d8b4fe',
  14: '#67e8f9',
  15: '#ffffff',
}

function renderSpans(line: ParsedLogLine) {
  if (!line.spans || line.spans.length === 0) {
    return line.plain
  }
  return line.spans.map((span: AnsiSpan, idx: number) => (
    <span
      key={idx}
      style={{
        color: span.fg !== undefined ? ANSI_COLORS[span.fg] : undefined,
        fontWeight: span.bold ? 600 : undefined,
        opacity: span.dim ? 0.65 : undefined,
      }}
    >
      {span.text}
    </span>
  ))
}

interface LogStreamViewProps {
  mode: 'app' | 'deploy'
  serverId?: string
  appId?: string
  deploymentId?: string
  title?: string
}

const MAX_UI_LINES = 25_000

export function LogStreamView({ mode, serverId, appId, deploymentId, title }: LogStreamViewProps) {
  const { prefs, updatePrefs } = usePreferences()
  const streamId = useMemo(
    () =>
      mode === 'deploy'
        ? `deploy-${serverId}-${deploymentId}`
        : `app-${serverId}-${appId}`,
    [appId, deploymentId, mode, serverId],
  )

  const [lines, setLines] = useState<ParsedLogLine[]>([])
  const [totalBuffered, setTotalBuffered] = useState(0)
  const [droppedTotal, setDroppedTotal] = useState(0)
  const [status, setStatus] = useState<string>('idle')
  const [filter, setFilter] = useState('')
  const [streamFilter, setStreamFilter] = useState<'all' | 'stdout' | 'stderr' | 'deploy'>('all')
  const [following, setFollowing] = useState(true)
  const [paused, setPaused] = useState(false)

  const pausedRef = useRef(false)
  pausedRef.current = paused
  const followingRef = useRef(true)
  followingRef.current = following
  const scrollParentRef = useRef<HTMLDivElement>(null)

  const visibleLines = useMemo(() => {
    const q = filter.trim().toLowerCase()
    if (!q && streamFilter === 'all') return lines
    return lines.filter((l) => {
      if (streamFilter !== 'all' && l.stream !== streamFilter) return false
      if (q && !l.plain.toLowerCase().includes(q)) return false
      return true
    })
  }, [filter, lines, streamFilter])

  const lineHeight = Math.round(prefs.logFontSize * 1.65)

  const rowVirtualizer = useVirtualizer({
    count: visibleLines.length,
    getScrollElement: () => scrollParentRef.current,
    estimateSize: () => lineHeight,
    overscan: 25,
  })

  const handleFrame = useCallback((frame: LogChannelFrame) => {
    if (frame.type === 'status') {
      setStatus(frame.state)
      if (frame.state === 'error' && frame.message) {
        toast.error(frame.message)
      }
      return
    }
    if (frame.type === 'batch') {
      setTotalBuffered(frame.total_buffered)
      setDroppedTotal(frame.dropped_total)
      if (pausedRef.current) return
      setLines((prev) => {
        const next = prev.concat(frame.lines)
        return next.length > MAX_UI_LINES ? next.slice(next.length - MAX_UI_LINES) : next
      })
    }
  }, [])

  // Auto-start live app or deploy log stream.
  useEffect(() => {
    if (mode === 'app') {
      if (!serverId || !appId) return
      setLines([])
      setTotalBuffered(0)
      setDroppedTotal(0)
      setStatus('connecting')
      void desktopApi.startAppLogs(serverId, appId, streamId, handleFrame, 500)
      return () => {
        void desktopApi.stopLogStream(streamId)
      }
    }
    if (mode === 'deploy') {
      if (!serverId || !deploymentId) return
      setLines([])
      setTotalBuffered(0)
      setDroppedTotal(0)
      setStatus('connecting')
      void desktopApi.startDeployLogs(serverId, deploymentId, streamId, handleFrame)
      return () => {
        void desktopApi.stopLogStream(streamId)
      }
    }
  }, [appId, deploymentId, handleFrame, mode, serverId, streamId])

  // Auto-scroll to bottom when following.
  useEffect(() => {
    if (followingRef.current && visibleLines.length > 0) {
      rowVirtualizer.scrollToIndex(visibleLines.length - 1, { align: 'end' })
    }
  }, [rowVirtualizer, visibleLines.length])

  // Run Rust-side search across the entire 50k ring buffer when user presses Enter in filter box.
  async function searchRustRing() {
    const res = await desktopApi.queryLogBuffer({
      streamId,
      filter: filter || undefined,
      streamFilter,
      offset: 0,
      limit: 2000,
    })
    setLines(res.lines)
    toast.info(
      `Ring query: ${res.matched_count.toLocaleString()} matches in ${res.total_buffered.toLocaleString()} buffered lines`,
    )
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-xl border border-line bg-surface">
      {/* Toolbar */}
      <div className="flex flex-wrap items-center gap-2 border-b border-line bg-surface-2/60 px-3 py-2 text-xs">
        <div className="flex items-center gap-2">
          <span
            className={`size-2 rounded-full ${
              status === 'streaming'
                ? 'bg-green-400'
                : status === 'connecting'
                  ? 'bg-amber-400'
                  : 'bg-zinc-500'
            }`}
          />
          <span className="font-medium text-fg">{title ?? 'Live Logs'}</span>
          <span className="font-mono text-[11px] text-muted tabular-nums">
            {visibleLines.length.toLocaleString()} shown · {totalBuffered.toLocaleString()} buffered
            {droppedTotal > 0 ? ` · ${droppedTotal.toLocaleString()} rotated` : ''}
          </span>
        </div>

        <div className="ml-auto flex flex-wrap items-center gap-1.5">
          <div className="relative">
            <Search className="pointer-events-none absolute top-1/2 left-2 size-3 -translate-y-1/2 text-muted" />
            <input
              type="text"
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') void searchRustRing()
              }}
              placeholder="Filter (Enter = full ring)"
              className="w-44 rounded-md border border-line bg-bg py-1 pr-6 pl-6 text-[11px] text-fg outline-none focus:border-emerald-500"
            />
            {filter && (
              <button
                type="button"
                onClick={() => setFilter('')}
                className="absolute top-1/2 right-1.5 -translate-y-1/2 text-muted hover:text-fg"
              >
                <X className="size-3" />
              </button>
            )}
          </div>

          <select
            value={streamFilter}
            onChange={(e) => setStreamFilter(e.target.value as typeof streamFilter)}
            className="rounded-md border border-line bg-bg px-2 py-1 text-[11px] text-fg"
          >
            <option value="all">All streams</option>
            <option value="stdout">stdout</option>
            <option value="stderr">stderr</option>
            <option value="deploy">deploy</option>
          </select>

          <button
            type="button"
            onClick={() => updatePrefs({ logWrap: !prefs.logWrap })}
            title={prefs.logWrap ? 'Disable line wrap' : 'Wrap long log lines'}
            className={`rounded-md border px-1.5 py-1 text-[11px] ${
              prefs.logWrap
                ? 'border-emerald-500/50 bg-emerald-500/15 text-emerald-300'
                : 'border-line bg-bg text-muted hover:bg-surface-2 hover:text-fg'
            }`}
          >
            <WrapText className="size-3.5" />
          </button>

          <button
            type="button"
            onClick={() => setPaused((p) => !p)}
            title={paused ? 'Resume UI updates' : 'Pause UI updates'}
            className="inline-flex items-center gap-1 rounded-md border border-line bg-bg px-2 py-1 text-[11px] text-fg hover:bg-surface-2"
          >
            {paused ? <Play className="size-3" /> : <Pause className="size-3" />}
            {paused ? 'Resume' : 'Pause'}
          </button>

          <button
            type="button"
            onClick={() => {
              const text = visibleLines.map((l) => l.plain).join('\n')
              void navigator.clipboard.writeText(text)
              toast.success(`Copied ${visibleLines.length.toLocaleString()} lines`)
            }}
            title="Copy visible lines"
            className="rounded-md border border-line bg-bg p-1 text-muted hover:bg-surface-2 hover:text-fg"
          >
            <Copy className="size-3.5" />
          </button>
        </div>
      </div>

      {/* Virtualized Log Pane */}
      <div className="relative min-h-0 flex-1">
        <div
          ref={scrollParentRef}
          onScroll={(e) => {
            const el = e.currentTarget
            const nearBottom = el.scrollHeight - el.scrollTop - el.clientHeight <= 40
            setFollowing(nearBottom)
          }}
          style={{ fontSize: `${prefs.logFontSize}px`, lineHeight: `${lineHeight}px` }}
          className="h-full overflow-auto bg-[#0b0c0f] py-1.5 font-mono text-zinc-200"
        >
          {visibleLines.length === 0 ? (
            <div className="px-4 py-6 font-sans text-xs text-muted">Waiting for log output…</div>
          ) : (
            <div
              style={{
                height: `${rowVirtualizer.getTotalSize()}px`,
                width: '100%',
                position: 'relative',
              }}
            >
              {rowVirtualizer.getVirtualItems().map((vRow) => {
                const line = visibleLines[vRow.index]
                return (
                  <div
                    key={vRow.key}
                    data-index={vRow.index}
                    ref={prefs.logWrap ? rowVirtualizer.measureElement : undefined}
                    style={{
                      position: 'absolute',
                      top: 0,
                      left: 0,
                      width: '100%',
                      minHeight: `${vRow.size}px`,
                      transform: `translateY(${vRow.start}px)`,
                    }}
                    className={`flex gap-3 border-l-2 px-3 hover:bg-white/[0.04] ${
                      prefs.logWrap ? 'whitespace-pre-wrap break-all' : 'whitespace-pre'
                    } ${
                      line.stream === 'stderr'
                        ? 'border-l-rose-500/70 bg-rose-500/[0.04]'
                        : line.stream === 'deploy'
                          ? 'border-l-amber-400/70'
                          : 'border-l-transparent'
                    }`}
                  >
                    <span className="w-[62px] shrink-0 text-zinc-500 tabular-nums select-none">
                      {formatLogTime(line.ts)}
                    </span>
                    <span className={`min-w-0 ${prefs.logWrap ? 'flex-1' : 'truncate'}`}>
                      {renderSpans(line)}
                    </span>
                  </div>
                )
              })}
            </div>
          )}
        </div>

        {!following && visibleLines.length > 0 && (
          <button
            type="button"
            onClick={() => {
              setFollowing(true)
              rowVirtualizer.scrollToIndex(visibleLines.length - 1, { align: 'end' })
            }}
            className="absolute right-4 bottom-3 inline-flex items-center gap-1.5 rounded-lg border border-line bg-surface-2 px-3 py-1.5 text-xs font-medium text-fg shadow-lg hover:bg-surface"
          >
            <ArrowDownToLine className="size-3.5" />
            Jump to latest
          </button>
        )}
      </div>
    </div>
  )
}
