import '@xterm/xterm/css/xterm.css'
import { FitAddon } from '@xterm/addon-fit'
import { Terminal } from '@xterm/xterm'
import { Plus, Terminal as TerminalIcon, X } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { toast } from 'sonner'
import { desktopApi } from '@/lib/ipc'
import { usePreferences } from '@/lib/preferences'

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
      lineHeight: 1.3,
      scrollback: 5000,
      theme: {
        background: '#0b0c0f',
        foreground: '#e4e4e7',
        cursor: '#10b981',
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
      className={`h-full w-full bg-[#0b0c0f] p-2 ${active ? 'block' : 'hidden'}`}
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
      <div className="flex flex-1 flex-col items-center justify-center rounded-xl border border-line bg-surface p-8 text-center">
        <TerminalIcon className="mb-2 size-7 text-muted" />
        <div className="text-sm font-medium text-fg">Container is not running</div>
        <p className="mt-1 text-xs text-muted">
          Start or deploy <span className="font-medium text-fg">{appName}</span> before opening an
          interactive shell.
        </p>
      </div>
    )
  }

  return (
    <div className="flex min-h-[420px] flex-1 flex-col overflow-hidden rounded-xl border border-line bg-surface">
      <div className="flex items-center gap-1 border-b border-line bg-surface-2/60 px-2 py-1.5">
        {tabs.map((tab) => (
          <div
            key={tab.id}
            onClick={() => setActiveId(tab.id)}
            className={`group flex cursor-pointer items-center gap-2 rounded-md px-2.5 py-1 text-xs ${
              tab.id === activeId
                ? 'bg-bg text-fg ring-1 ring-line'
                : 'text-muted hover:bg-bg/50 hover:text-fg'
            }`}
          >
            <span
              className={`size-1.5 rounded-full ${
                tab.state === 'connected'
                  ? 'bg-emerald-400'
                  : tab.state === 'connecting'
                    ? 'bg-amber-400'
                    : 'bg-zinc-500'
              }`}
            />
            <span className="font-mono">{tab.label}</span>
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation()
                closeTab(tab.id)
              }}
              className="rounded p-0.5 text-muted hover:bg-surface-2 hover:text-fg"
            >
              <X className="size-3" />
            </button>
          </div>
        ))}
        <button
          type="button"
          onClick={openNewTab}
          className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-xs text-muted hover:bg-bg hover:text-fg"
        >
          <Plus className="size-3.5" />
          New shell
        </button>
      </div>

      <div className="relative min-h-0 flex-1 bg-[#0b0c0f]">
        {tabs.length === 0 ? (
          <div className="flex h-full flex-col items-center justify-center p-8 text-center">
            <TerminalIcon className="mb-2 size-7 text-emerald-400" />
            <div className="text-sm font-medium text-fg">On-demand Container Terminal</div>
            <p className="mt-1 max-w-sm text-xs text-muted">
              WebSockets are opened through the native Rust core only when you launch a shell — zero
              idle connections.
            </p>
            <button
              type="button"
              onClick={openNewTab}
              className="mt-4 inline-flex items-center gap-1.5 rounded-lg bg-emerald-600 px-4 py-2 text-xs font-medium text-white hover:bg-emerald-500"
            >
              <TerminalIcon className="size-3.5" />
              Open Container Shell
            </button>
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
