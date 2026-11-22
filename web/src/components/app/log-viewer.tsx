import { ArrowDownToLine, Search, WrapText, X } from 'lucide-react'
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { CopyButton } from '@/components/app/copy-button'
import { Input } from '@/components/fluid/input'
import { Tooltip } from '@/components/fluid/tooltip'
import { Button } from '@/components/ui/button'
import type { LogLine } from '@/hooks/use-log-stream'
import { logTime } from '@/lib/format'
import { usePreferences } from '@/lib/preferences'
import { cn } from '@/lib/utils'

/** How close to the bottom still counts as "following". */
const STICKY_PX = 32

/**
 * A terminal-style log pane. Follows new output while the reader is at the
 * bottom and stops the moment they scroll up.
 */
export function LogViewer({
  lines,
  status,
  empty,
  className,
}: {
  lines: LogLine[]
  /** Shown at the left of the toolbar, e.g. a live indicator. */
  status?: ReactNode
  /** Shown when there are no lines. */
  empty?: ReactNode
  className?: string
}) {
  const pane = useRef<HTMLDivElement>(null)
  const following = useRef(true)
  const { prefs, updatePrefs } = usePreferences()
  const [atBottom, setAtBottom] = useState(true)
  const [filter, setFilter] = useState('')
  const wrap = prefs.logWrap
  const fontSize = prefs.logFontSize

  const visible = useMemo(() => {
    const q = filter.trim().toLowerCase()
    if (!q) return lines
    return lines.filter((l) => l.line.toLowerCase().includes(q))
  }, [lines, filter])

  useLayoutEffect(() => {
    const el = pane.current
    if (el && following.current) el.scrollTop = el.scrollHeight
  }, [visible, wrap])

  useEffect(() => {
    const el = pane.current
    if (!el) return
    const onScroll = () => {
      const bottom = el.scrollHeight - el.scrollTop - el.clientHeight <= STICKY_PX
      following.current = bottom
      setAtBottom(bottom)
    }
    el.addEventListener('scroll', onScroll, { passive: true })
    return () => el.removeEventListener('scroll', onScroll)
  }, [])

  function jumpToEnd() {
    const el = pane.current
    if (!el) return
    following.current = true
    el.scrollTop = el.scrollHeight
    setAtBottom(true)
  }

  return (
    <div
      className={cn(
        'relative flex min-h-0 flex-col overflow-hidden rounded-xl bg-surface shadow-[var(--shadow-2)]',
        className,
      )}
    >
      <div className="flex min-h-10 shrink-0 flex-wrap items-center gap-2 border-b border-border/80 bg-card px-3 py-1.5">
        <div className="min-w-0 flex-1 text-[13px] text-muted-foreground">{status}</div>
        <div className="relative w-40 sm:w-52">
          <Search className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground" />
          <Input
            size="compact"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            placeholder="Filter logs"
            aria-label="Filter log output"
            className="pl-7 pr-7 text-xs"
          />
          {filter ? (
            <button
              type="button"
              onClick={() => setFilter('')}
              aria-label="Clear log filter"
              className="absolute top-1/2 right-2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
            >
              <X className="size-3.5" />
            </button>
          ) : null}
        </div>
        <span className="text-xs text-muted-foreground tabular-nums">
          {filter ? `${visible.length} / ${lines.length}` : lines.length.toLocaleString()}{' '}
          {lines.length === 1 ? 'line' : 'lines'}
        </span>
        <Tooltip content={wrap ? 'Disable line wrap' : 'Wrap long lines'}>
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={wrap ? 'Turn off line wrapping' : 'Wrap long lines'}
            aria-pressed={wrap}
            onClick={() => updatePrefs({ logWrap: !wrap })}
            className={wrap ? 'text-foreground' : 'text-muted-foreground'}
          >
            <WrapText />
          </Button>
        </Tooltip>
        <CopyButton value={visible.map((l) => l.line).join('\n')} label="Copy all lines" />
      </div>
      <div
        ref={pane}
        role="log"
        aria-label="Log output"
        tabIndex={0}
        style={{ fontSize: `${fontSize}px`, lineHeight: `${fontSize + 8}px` }}
        className="min-h-0 flex-1 overflow-auto py-2 font-mono outline-none focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:ring-inset"
      >
        {visible.length === 0 ? (
          <p className="px-3 font-sans text-[13px] text-muted-foreground">
            {filter ? 'No log lines match that filter.' : (empty ?? 'No output yet.')}
          </p>
        ) : (
          visible.map((l) => (
            <div
              key={l.seq}
              className={cn(
                'flex gap-3 border-l-2 border-transparent px-3 hover:bg-hover',
                l.stream === 'stderr' && 'border-l-fail/55 bg-fail/[0.035]',
                l.stream === 'deploy' && 'border-l-busy/55',
              )}
            >
              <time className="shrink-0 text-muted-foreground/75 select-none">{logTime(l.ts)}</time>
              <span
                className={cn(
                  'min-w-0',
                  wrap ? 'break-words whitespace-pre-wrap' : 'whitespace-pre',
                  l.stream === 'stderr' && 'text-foreground/95',
                  l.stream === 'deploy' && 'text-busy',
                )}
              >
                {l.line}
              </span>
            </div>
          ))
        )}
      </div>
      {atBottom ? null : (
        <Button size="sm" variant="secondary" onClick={jumpToEnd} className="absolute right-4 bottom-3 shadow-sm">
          <ArrowDownToLine data-icon="inline-start" />
          Latest
        </Button>
      )}
    </div>
  )
}
