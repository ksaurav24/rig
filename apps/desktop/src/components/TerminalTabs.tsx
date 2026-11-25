import '@xterm/xterm/css/xterm.css'
import { FitAddon } from '@xterm/addon-fit'
import { Terminal } from '@xterm/xterm'
import { Plus, Terminal as TerminalIcon, X } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { toast } from 'sonner'
import { EmptyState } from '@rig/ui/app/state'
import { Button } from '@rig/ui/button'
import { desktopApi } from '@/lib/ipc'
import { usePreferences } from '@/lib/preferences'
import { cn } from '@rig/ui/lib/utils'

interface TerminalTab {
  id: string
  label: string
  state: string
}

interface SingleTerminalPaneProps {
  serverId: string
  appId: string
  sessionId: string
  active: boolean
  onStateChange: (state: string) => void
}

function SingleTerminalPane({
  serverId,
  appId,
  sessionId,
  active,
  fontSize,
  onStateChange,
}: SingleTerminalPaneProps & { fontSize: number }) {
  const hostRef = useRef<HTMLDivElement>(null)
  const fitRef = useRef<FitAddon | null>(null)
  const termRef = useRef<Terminal | null>(null)

  useEffect(() => {
    const container = hostRef.current
    if (!container) return

    const term = new Terminal({
      cursorBlink: true,
      fontFamily: "'JetBrains Mono Variable', ui-monospace, monospace",
      fontSize,
      lineHeight: 1.35,
      scrollback: 5000,
      theme: {
        background: '#121110',
        foreground: '#e8e6e3',
        cursor: '#34d399',
        selectionBackground: 'rgba(16, 185, 129, 0.28)',
      },
    })
    const fit = new FitAddon()
    term.loadAddon(fit)
    term.open(container)
    fit.fit()
    termRef.current = term
    fitRef.current = fit

    const encoder = new TextEncoder()
    const dataDisp = term.onData((text) => {
      const bytes = Array.from(encoder.encode(text))
      void desktopApi.writeTerminal(sessionId, bytes)
    })

    const resizeDisp = term.onResize(({ cols, rows }) => {
      void desktopApi.resizeTerminal(sessionId, cols, rows)
    })

    const ro = new ResizeObserver(() => {
      fit.fit()
    })
    ro.observe(container)

    onStateChange('connecting')
    void desktopApi
      .openTerminal(serverId, appId, sessionId, term.cols || 80, term.rows || 24, (frame) => {
        if (frame.type === 'output') {
          term.write(new Uint8Array(frame.data))
        } else if (frame.type === 'state') {
          onStateChange(frame.state)
          if (frame.state === 'error' && frame.message) {
            term.writeln(`\r\n\x1b[31m[terminal error: ${frame.message}]\x1b[0m`)
          } else if (frame.state === 'closed') {
            term.writeln('\r\n\x1b[90m[session closed]\x1b[0m')
          }
        }
      })
      .catch((err) => {
        onStateChange('error')
        toast.error(String(err))
        term.writeln(`\x1b[31mFailed to open terminal: ${String(err)}\x1b[0m`)
      })

    return () => {
      ro.disconnect()
      dataDisp.dispose()
      resizeDisp.dispose()
      void desktopApi.closeTerminal(sessionId)
      term.dispose()
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [appId, serverId, sessionId])

  useEffect(() => {
    if (termRef.current) {
      termRef.current.options.fontSize = fontSize
      fitRef.current?.fit()
    }
  }, [fontSize])

  useEffect(() => {
    if (active) {
      fitRef.current?.fit()
      termRef.current?.focus()
    }
  }, [active])

  return (
    <div
      ref={hostRef}
      className={cn('h-full w-full bg-[#121110] p-2.5', active ? 'block' : 'hidden')}
    />
  )
}

interface TerminalTabsProps {
  serverId: string
  appId: string
  appName: string
  isRunning: boolean
}

export function TerminalTabs({ serverId, appId, appName, isRunning }: TerminalTabsProps) {
  const { prefs } = usePreferences()
  const [tabs, setTabs] = useState<TerminalTab[]>([])
  const [activeId, setActiveId] = useState<string | null>(null)
  const counterRef = useRef(1)

  function openNewTab() {
    const n = counterRef.current++
    const id = `term-${serverId}-${appId}-${n}-${Date.now()}`
    setTabs((prev) => [...prev, { id, label: `shell #${n}`, state: 'connecting' }])
    setActiveId(id)
  }

  function closeTab(id: string) {
    void desktopApi.closeTerminal(id)
    setTabs((prev) => {
      const next = prev.filter((t) => t.id !== id)
      if (activeId === id) {
        setActiveId(next[next.length - 1]?.id ?? null)
      }
      return next
    })
  }

  if (!isRunning) {
    return (
      <EmptyState icon={TerminalIcon} title="Container is not running">
        Start or deploy <span className="font-medium text-foreground">{appName}</span> before
        opening an interactive shell.
      </EmptyState>
    )
  }

  return (
    <div className="flex min-h-[440px] flex-1 flex-col overflow-hidden rounded-xl bg-card shadow-[var(--shadow-2)]">
      <div className="flex items-center gap-1.5 border-b border-border/80 bg-surface/60 px-3 py-2">
        {tabs.map((tab) => (
          <div
            key={tab.id}
            onClick={() => setActiveId(tab.id)}
            className={cn(
              'group flex cursor-pointer items-center gap-2 rounded-lg px-3 py-1.5 text-[13px] transition-colors',
              tab.id === activeId
                ? 'bg-active font-medium text-foreground'
                : 'text-muted-foreground hover:bg-hover hover:text-foreground',
            )}
          >
            <span
              className={cn(
                'size-2 rounded-full',
                tab.state === 'connected'
                  ? 'bg-ok'
                  : tab.state === 'connecting'
                    ? 'bg-warn animate-pulse'
                    : 'bg-idle',
              )}
            />
            <span className="font-mono">{tab.label}</span>
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation()
                closeTab(tab.id)
              }}
              aria-label={`Close ${tab.label}`}
              className="rounded p-0.5 text-muted-foreground hover:bg-hover hover:text-foreground"
            >
              <X className="size-3.5" />
            </button>
          </div>
        ))}
        <Button type="button" variant="ghost" size="compact" leadingIcon={Plus} onClick={openNewTab}>
          New shell
        </Button>
      </div>

      <div className="relative min-h-0 flex-1 bg-[#121110]">
        {tabs.length === 0 ? (
          <div className="flex h-full flex-col items-center justify-center p-10 text-center">
            <div className="mb-3.5 flex size-12 items-center justify-center rounded-xl bg-white/5 text-primary">
              <TerminalIcon className="size-6" strokeWidth={1.6} />
            </div>
            <div className="text-[15px] font-medium text-zinc-100">
              On-demand Container Terminal
            </div>
            <p className="mt-1.5 max-w-sm text-[13.5px] leading-relaxed text-zinc-400">
              WebSockets are opened through the native Rust core only when you launch a shell — zero
              idle connections.
            </p>
            <Button
              type="button"
              variant="primary"
              leadingIcon={TerminalIcon}
              onClick={openNewTab}
              className="mt-5"
            >
              Open Container Shell
            </Button>
          </div>
        ) : (
          tabs.map((tab) => (
            <SingleTerminalPane
              key={tab.id}
              serverId={serverId}
              appId={appId}
              sessionId={tab.id}
              active={tab.id === activeId}
              fontSize={prefs.logFontSize}
              onStateChange={(st) =>
                setTabs((prev) => prev.map((t) => (t.id === tab.id ? { ...t, state: st } : t)))
              }
            />
          ))
        )}
      </div>
    </div>
  )
}
