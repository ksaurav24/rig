import { FitAddon } from '@xterm/addon-fit'
import { Terminal as Xterm, type ITheme } from '@xterm/xterm'
import '@xterm/xterm/css/xterm.css'
import { RotateCw } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'

import { Status } from '@/components/app/state'
import { Button } from '@/components/ui/button'
import { usePreferences } from '@/lib/preferences'
import { cn } from '@/lib/utils'

type Phase = 'connecting' | 'open' | 'closed'

// ANSI colours picked to stay readable on our light and dark surfaces.
const ANSI: Record<'light' | 'dark', Partial<ITheme>> = {
  light: {
    black: '#1f2328', red: '#b3261e', green: '#1a7f37', yellow: '#8a5a00',
    blue: '#0b57d0', magenta: '#8250df', cyan: '#0f6e79', white: '#6e7781',
    brightBlack: '#57606a', brightRed: '#cf222e', brightGreen: '#2da44e', brightYellow: '#9a6700',
    brightBlue: '#218bff', brightMagenta: '#a475f9', brightCyan: '#1b7c83', brightWhite: '#3d444d',
  },
  dark: {
    black: '#3b4048', red: '#ff7b72', green: '#56d364', yellow: '#e3b341',
    blue: '#79c0ff', magenta: '#d2a8ff', cyan: '#56d4dd', white: '#c9d1d9',
    brightBlack: '#8b949e', brightRed: '#ffa198', brightGreen: '#7ee787', brightYellow: '#f2cc60',
    brightBlue: '#a5d6ff', brightMagenta: '#e2c5ff', brightCyan: '#87e8ee', brightWhite: '#f0f6fc',
  },
}

/** Resolves a design token (any CSS colour syntax) to the `#rrggbb` xterm wants. */
function token(name: string, fallback: string): string {
  const value = getComputedStyle(document.documentElement).getPropertyValue(name).trim()
  const ctx = document.createElement('canvas').getContext('2d', { willReadFrequently: true })
  if (!value || !ctx) return fallback
  ctx.fillStyle = fallback
  ctx.fillStyle = value
  ctx.fillRect(0, 0, 1, 1)
  const [r, g, b] = ctx.getImageData(0, 0, 1, 1).data
  return `#${[r, g, b].map((n) => n.toString(16).padStart(2, '0')).join('')}`
}

function currentTheme(): ITheme {
  const dark = document.documentElement.classList.contains('dark')
  const foreground = token('--foreground', dark ? '#e6e6e6' : '#1f2328')
  return {
    ...ANSI[dark ? 'dark' : 'light'],
    background: token('--surface', dark ? '#111111' : '#fafafa'),
    foreground,
    cursor: foreground,
    cursorAccent: token('--surface', dark ? '#111111' : '#fafafa'),
    selectionBackground: dark ? '#ffffff33' : '#00000026',
  }
}

/**
 * An interactive shell in an app's container, over the terminal WebSocket.
 * Binary frames carry terminal bytes both ways; a JSON text frame resizes.
 */
export default function Terminal({ appId, className }: { appId: string; className?: string }) {
  const host = useRef<HTMLDivElement>(null)
  const termRef = useRef<Xterm | null>(null)
  const fitRef = useRef<FitAddon | null>(null)
  const { prefs } = usePreferences()
  const [phase, setPhase] = useState<Phase>('connecting')
  const [reason, setReason] = useState('')
  const [attempt, setAttempt] = useState(0)

  useEffect(() => {
    const el = host.current
    if (!el) return
    setPhase('connecting')
    setReason('')

    const term = new Xterm({
      cursorBlink: true,
      fontFamily: getComputedStyle(document.documentElement).getPropertyValue('--font-mono') || 'monospace',
      fontSize: prefs.logFontSize,
      lineHeight: 1.25,
      scrollback: 5000,
      theme: currentTheme(),
    })
    const fit = new FitAddon()
    termRef.current = term
    fitRef.current = fit
    term.loadAddon(fit)
    term.open(el)
    fit.fit()

    const scheme = location.protocol === 'https:' ? 'wss' : 'ws'
    const socket = new WebSocket(
      `${scheme}://${location.host}/api/v1/apps/${encodeURIComponent(appId)}/terminal?cols=${term.cols}&rows=${term.rows}`,
    )
    socket.binaryType = 'arraybuffer'
    const encoder = new TextEncoder()
    let opened = false

    socket.onopen = () => {
      opened = true
      setPhase('open')
      term.focus()
    }
    socket.onmessage = (e) => {
      if (e.data instanceof ArrayBuffer) term.write(new Uint8Array(e.data))
    }
    socket.onclose = (e) => {
      setPhase('closed')
      // A refused upgrade gives the browser no status or body to show.
      setReason(e.reason || (opened ? 'The connection was lost' : 'Could not open a shell. Is the app still running?'))
    }

    const send = (data: string | Uint8Array<ArrayBuffer>) => {
      if (socket.readyState === WebSocket.OPEN) socket.send(data)
    }
    const input = term.onData((data) => send(encoder.encode(data)))
    const binary = term.onBinary((data) => send(Uint8Array.from(data, (c) => c.charCodeAt(0))))
    const resized = term.onResize(({ cols, rows }) => send(JSON.stringify({ type: 'resize', cols, rows })))

    // One fit per frame however often the layout changes.
    let frame = 0
    const sizes = new ResizeObserver(() => {
      frame ||= requestAnimationFrame(() => {
        frame = 0
        fit.fit()
      })
    })
    sizes.observe(el)

    const themes = new MutationObserver(() => {
      term.options.theme = currentTheme()
    })
    themes.observe(document.documentElement, { attributes: true, attributeFilter: ['class'] })

    return () => {
      cancelAnimationFrame(frame)
      sizes.disconnect()
      themes.disconnect()
      input.dispose()
      binary.dispose()
      resized.dispose()
      socket.onclose = null
      socket.close()
      term.dispose()
      termRef.current = null
      fitRef.current = null
    }
  }, [appId, attempt])

  useEffect(() => {
    const term = termRef.current
    if (!term) return
    term.options.fontSize = prefs.logFontSize
    fitRef.current?.fit()
  }, [prefs.logFontSize])

  return (
    <div className={cn('flex min-h-0 flex-col overflow-hidden rounded-xl bg-surface shadow-[var(--shadow-2)]', className)}>
      <div className="flex h-10 shrink-0 items-center gap-3 border-b border-border/80 bg-card pr-2 pl-3">
        <div className="min-w-0 flex-1 truncate text-[0.8125rem] text-muted-foreground" aria-live="polite">
          {phase === 'open' ? (
            <Status tone="ok">Connected</Status>
          ) : phase === 'connecting' ? (
            <Status tone="busy">Connecting</Status>
          ) : (
            <Status tone="idle">{reason}</Status>
          )}
        </div>
        {phase === 'closed' ? (
          <Button variant="ghost" size="sm" onClick={() => setAttempt((n) => n + 1)}>
            <RotateCw data-icon="inline-start" />
            New shell
          </Button>
        ) : null}
      </div>
      <div ref={host} className={cn('min-h-0 flex-1 p-2', phase === 'closed' && 'opacity-60')} />
    </div>
  )
}
