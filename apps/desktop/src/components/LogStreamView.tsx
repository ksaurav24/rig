import { useVirtualizer } from '@tanstack/react-virtual'
import { ArrowDownToLine, Copy, Pause, Play, Search, WrapText, X } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { toast } from 'sonner'
import { Status, statusTone } from '@rig/ui/app/state'
import { Button } from '@rig/ui/button'
import { Input } from '@rig/ui/input'
import { Select, SelectContent, SelectItem, SelectTrigger } from '@rig/ui/select'
import { formatLogTime } from '@/lib/format'
import { desktopApi } from '@/lib/ipc'
import { usePreferences } from '@/lib/preferences'
import type { AnsiSpan, LogChannelFrame, ParsedLogLine } from '@/lib/types'
import { cn } from '@rig/ui/lib/utils'

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

  useEffect(() => {
    if (followingRef.current && visibleLines.length > 0) {
      rowVirtualizer.scrollToIndex(visibleLines.length - 1, { align: 'end' })
    }
  }, [rowVirtualizer, visibleLines.length])

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
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-xl bg-card shadow-[var(--shadow-2)]">
      {/* Toolbar */}
      <div className="flex flex-wrap items-center gap-2.5 border-b border-border/80 bg-surface/60 px-4 py-2.5">
        <div className="flex items-center gap-2.5">
          <Status tone={statusTone(status)} size="compact">
            {title ?? 'Live Logs'}
          </Status>
          <span className="font-mono text-[12px] text-muted-foreground tabular-nums">
            {visibleLines.length.toLocaleString()} shown · {totalBuffered.toLocaleString()} buffered
            {droppedTotal > 0 ? ` · ${droppedTotal.toLocaleString()} rotated` : ''}
          </span>
        </div>

        <div className="ml-auto flex flex-wrap items-center gap-2">
          <div className="relative">
            <Search className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground" />
            <Input
              size="compact"
              type="text"
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') void searchRustRing()
              }}
              placeholder="Filter (Enter = full ring)"
              className="w-48 pr-7 pl-8"
            />
            {filter && (
              <button
                type="button"
                onClick={() => setFilter('')}
                aria-label="Clear filter"
                className="absolute top-1/2 right-2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
              >
                <X className="size-3.5" />
              </button>
            )}
          </div>

          <Select
            size="compact"
            value={streamFilter}
            onValueChange={(v) => setStreamFilter(v as typeof streamFilter)}
          >
            <SelectTrigger className="w-[132px]" />
            <SelectContent>
              <SelectItem index={0} value="all">All streams</SelectItem>
              <SelectItem index={1} value="stdout">stdout</SelectItem>
              <SelectItem index={2} value="stderr">stderr</SelectItem>
              <SelectItem index={3} value="deploy">deploy</SelectItem>
            </SelectContent>
          </Select>

          <Button
            type="button"
            variant="tertiary"
            size="icon-compact"
            active={prefs.logWrap}
            onClick={() => updatePrefs({ logWrap: !prefs.logWrap })}
            title={prefs.logWrap ? 'Disable line wrap' : 'Wrap long log lines'}
            aria-label={prefs.logWrap ? 'Disable line wrap' : 'Wrap long log lines'}
          >
            <WrapText />
          </Button>

          <Button
            type="button"
            variant="tertiary"
            size="compact"
            leadingIcon={paused ? Play : Pause}
            active={paused}
            onClick={() => setPaused((p) => !p)}
            title={paused ? 'Resume UI updates' : 'Pause UI updates'}
          >
            {paused ? 'Resume' : 'Pause'}
          </Button>

          <Button
            type="button"
            variant="tertiary"
            size="icon-compact"
            onClick={() => {
              const text = visibleLines.map((l) => l.plain).join('\n')
              void navigator.clipboard.writeText(text)
              toast.success(`Copied ${visibleLines.length.toLocaleString()} lines`)
            }}
            title="Copy visible lines"
            aria-label="Copy visible lines"
          >
            <Copy />
          </Button>
        </div>
      </div>

      {/* Virtualized Log Pane (no backdrop-filter per WebKitGTK perf discipline) */}
      <div className="relative min-h-0 flex-1">
        <div
          ref={scrollParentRef}
          onScroll={(e) => {
            const el = e.currentTarget
            const nearBottom = el.scrollHeight - el.scrollTop - el.clientHeight <= 40
            setFollowing(nearBottom)
          }}
          style={{ fontSize: `${prefs.logFontSize}px`, lineHeight: `${lineHeight}px` }}
          className="h-full overflow-auto bg-[#121110] py-2 font-mono text-zinc-200"
        >
          {visibleLines.length === 0 ? (
            <div className="px-5 py-8 font-sans text-[13.5px] text-zinc-400">
              Waiting for log output…
            </div>
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
                    className={cn(
                      'flex gap-3.5 border-l-2 px-4 hover:bg-white/[0.04]',
                      prefs.logWrap ? 'whitespace-pre-wrap break-all' : 'whitespace-pre',
                      line.stream === 'stderr'
                        ? 'border-l-rose-500/75 bg-rose-500/[0.05]'
                        : line.stream === 'deploy'
                          ? 'border-l-amber-400/75'
                          : 'border-l-transparent',
                    )}
                  >
                    <span className="w-[68px] shrink-0 text-zinc-500 tabular-nums select-none">
                      {formatLogTime(line.ts)}
                    </span>
                    <span className={cn('min-w-0', prefs.logWrap ? 'flex-1' : 'truncate')}>
                      {renderSpans(line)}
                    </span>
                  </div>
                )
              })}
            </div>
          )}
        </div>

        {!following && visibleLines.length > 0 && (
          <Button
            type="button"
            variant="primary"
            size="compact"
            leadingIcon={ArrowDownToLine}
            onClick={() => {
              setFollowing(true)
              rowVirtualizer.scrollToIndex(visibleLines.length - 1, { align: 'end' })
            }}
            className="absolute right-4 bottom-4 shadow-[var(--shadow-4)]"
          >
            Jump to latest
          </Button>
        )}
      </div>
    </div>
  )
}
