import { ArrowDownToLine, WrapText } from 'lucide-react'
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react'
import { CopyButton } from '@/components/app/copy-button'
import { Button } from '@/components/ui/button'
import type { LogLine } from '@/hooks/use-log-stream'
import { cn } from '@/lib/utils'

const TIME = new Intl.DateTimeFormat(undefined, { hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false })

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
  const [atBottom, setAtBottom] = useState(true)
  const [wrap, setWrap] = useState(true)

  useLayoutEffect(() => {
    const el = pane.current
    if (el && following.current) el.scrollTop = el.scrollHeight
  }, [lines, wrap])

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
    <div className={cn('relative flex min-h-0 flex-col overflow-hidden rounded-lg border bg-surface', className)}>
      <div className="flex h-9 shrink-0 items-center gap-1 border-b bg-card px-3">
        <div className="min-w-0 flex-1 text-[0.8125rem] text-muted-foreground">{status}</div>
        <span className="mr-1 text-xs text-muted-foreground tabular-nums">
          {lines.length.toLocaleString()} {lines.length === 1 ? 'line' : 'lines'}
        </span>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={wrap ? 'Turn off line wrapping' : 'Wrap long lines'}
          aria-pressed={wrap}
          onClick={() => setWrap((w) => !w)}
          className={wrap ? 'text-foreground' : 'text-muted-foreground'}
        >
          <WrapText />
        </Button>
        <CopyButton value={lines.map((l) => l.line).join('\n')} label="Copy all lines" />
      </div>
      <div
        ref={pane}
        role="log"
        aria-label="Log output"
        tabIndex={0}
        className="min-h-0 flex-1 overflow-auto py-2 font-mono text-[0.75rem] leading-5 outline-none focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:ring-inset"
      >
        {lines.length === 0 ? (
          <p className="px-3 font-sans text-[0.8125rem] text-muted-foreground">{empty ?? 'No output yet.'}</p>
        ) : (
          lines.map((l) => (
            <div key={l.seq} className="flex gap-3 px-3 hover:bg-foreground/[0.04]">
              <time className="shrink-0 text-muted-foreground/70 select-none">{TIME.format(new Date(l.ts))}</time>
              <span
                className={cn(
                  'min-w-0',
                  wrap ? 'break-words whitespace-pre-wrap' : 'whitespace-pre',
                  l.stream === 'stderr' && 'text-fail',
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
