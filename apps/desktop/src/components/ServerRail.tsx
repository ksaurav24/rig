import { Moon, Palette, Plus, Sun } from 'lucide-react'
import { useState } from 'react'
import { accentHex, initials } from '@/lib/format'
import type { ServerHealth, ServerView } from '@/lib/types'

const HEALTH_DOT: Record<ServerHealth, string> = {
  online: 'bg-green-400',
  deploying: 'bg-amber-400',
  connecting: 'bg-sky-400',
  degraded: 'bg-rose-500',
  offline: 'bg-zinc-500',
}

interface ServerRailProps {
  servers: ServerView[]
  activeId: string | null
  onSelect: (id: string) => void
  onAdd: () => void
  onReorder: (orderedIds: string[]) => void
  onOpenAppearance: () => void
  theme: 'dark' | 'light'
  onToggleTheme: () => void
}

export function ServerRail({
  servers,
  activeId,
  onSelect,
  onAdd,
  onReorder,
  onOpenAppearance,
  theme,
  onToggleTheme,
}: ServerRailProps) {
  const [dragId, setDragId] = useState<string | null>(null)
  const [overId, setOverId] = useState<string | null>(null)

  function handleDrop(targetId: string) {
    if (!dragId || dragId === targetId) {
      setDragId(null)
      setOverId(null)
      return
    }
    const ids = servers.map((s) => s.id)
    const from = ids.indexOf(dragId)
    const to = ids.indexOf(targetId)
    if (from === -1 || to === -1) return
    ids.splice(from, 1)
    ids.splice(to, 0, dragId)
    onReorder(ids)
    setDragId(null)
    setOverId(null)
  }

  return (
    <aside
      aria-label="Rig servers"
      className="flex w-[66px] shrink-0 flex-col items-center border-r border-line bg-rail py-3 select-none"
    >
      <div className="mb-3 flex size-9 items-center justify-center rounded-xl bg-surface text-xs font-bold tracking-tight text-fg ring-1 ring-line">
        <span className="text-emerald-400">R</span>ig
      </div>

      <div className="flex min-h-0 flex-1 flex-col items-center gap-2 overflow-y-auto px-2 py-1">
        {servers.map((server, idx) => {
          const active = server.id === activeId
          const color = accentHex(server.accent)
          const isDropTarget = overId === server.id && dragId !== server.id
          return (
            <div
              key={server.id}
              draggable
              onDragStart={() => setDragId(server.id)}
              onDragOver={(e) => {
                e.preventDefault()
                if (overId !== server.id) setOverId(server.id)
              }}
              onDragLeave={() => {
                if (overId === server.id) setOverId(null)
              }}
              onDrop={(e) => {
                e.preventDefault()
                handleDrop(server.id)
              }}
              onDragEnd={() => {
                setDragId(null)
                setOverId(null)
              }}
              className="relative flex w-full items-center justify-center"
            >
              {active && (
                <span
                  className="absolute -left-2 h-6 w-1 rounded-r-full"
                  style={{ backgroundColor: color }}
                />
              )}
              <button
                type="button"
                onClick={() => onSelect(server.id)}
                title={`${server.name} (${server.url}) · ${server.health}${idx < 9 ? ` · Ctrl+${idx + 1}` : ''}`}
                className={`group relative flex size-10 items-center justify-center rounded-xl text-xs font-semibold transition-colors ${
                  active
                    ? 'text-white ring-2 ring-offset-2 ring-offset-rail'
                    : 'bg-surface text-fg/85 ring-1 ring-line hover:bg-surface-2 hover:text-fg'
                } ${isDropTarget ? 'ring-2 ring-emerald-400' : ''}`}
                style={
                  active
                    ? { backgroundColor: color, boxShadow: `0 0 0 1px ${color}` }
                    : undefined
                }
              >
                <span>{initials(server.name)}</span>
                <span
                  className={`absolute -right-0.5 -bottom-0.5 size-3 rounded-full border-2 border-rail ${HEALTH_DOT[server.health]}`}
                />
                {idx < 9 && (
                  <span className="pointer-events-none absolute -top-1 -left-1 hidden rounded bg-surface-2 px-1 text-[9px] font-mono text-muted ring-1 ring-line group-hover:block">
                    {idx + 1}
                  </span>
                )}
              </button>
            </div>
          )
        })}

        <button
          type="button"
          onClick={onAdd}
          title="Add Rig server (Ctrl+N)"
          className="mt-1 flex size-10 items-center justify-center rounded-xl border border-dashed border-line text-muted transition-colors hover:border-emerald-500/60 hover:bg-surface hover:text-fg"
        >
          <Plus className="size-4" />
        </button>
      </div>

      <div className="mt-2 flex flex-col items-center gap-1.5 border-t border-line pt-2.5">
        <button
          type="button"
          onClick={onOpenAppearance}
          title="Customize UI & Appearance"
          className="flex size-9 items-center justify-center rounded-lg text-muted transition-colors hover:bg-surface hover:text-emerald-400"
        >
          <Palette className="size-4" />
        </button>
        <button
          type="button"
          onClick={onToggleTheme}
          title={theme === 'dark' ? 'Switch to light theme' : 'Switch to dark theme'}
          className="flex size-9 items-center justify-center rounded-lg text-muted transition-colors hover:bg-surface hover:text-fg"
        >
          {theme === 'dark' ? <Sun className="size-4" /> : <Moon className="size-4" />}
        </button>
      </div>
    </aside>
  )
}
