import { Moon, Plus, Settings, Sun } from 'lucide-react'
import { motion } from 'motion/react'
import { useState } from 'react'
import { Logo } from '@rig/ui/app/logo'
import { Tooltip, TooltipGroup } from '@rig/ui/tooltip'
import { accentHex, initials } from '@/lib/format'
import { useShape } from '@rig/ui/lib/shape-context'
import { spring } from '@rig/ui/lib/springs'
import type { ServerHealth, ServerView } from '@/lib/types'
import { cn } from '@rig/ui/lib/utils'

const HEALTH_DOT: Record<ServerHealth, string> = {
  online: 'bg-ok',
  deploying: 'bg-warn animate-pulse',
  connecting: 'bg-busy animate-pulse',
  degraded: 'bg-fail',
  offline: 'bg-idle',
}

interface ServerRailProps {
  servers: ServerView[]
  activeId: string | null
  appSettingsOpen: boolean
  onSelect: (id: string) => void
  onAdd: () => void
  onReorder: (orderedIds: string[]) => void
  onOpenAppSettings: () => void
  theme: 'dark' | 'light'
  onToggleTheme: () => void
}

export function ServerRail({
  servers,
  activeId,
  appSettingsOpen,
  onSelect,
  onAdd,
  onReorder,
  onOpenAppSettings,
  theme,
  onToggleTheme,
}: ServerRailProps) {
  const shape = useShape()
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
      className="flex w-[68px] shrink-0 flex-col items-center border-r border-sidebar-border bg-rail py-3.5 select-none"
    >
      <div className="mb-3.5 flex size-10 items-center justify-center rounded-xl bg-card shadow-[var(--shadow-2)]">
        <Logo className="size-7" />
      </div>

      <TooltipGroup>
        <div className="flex min-h-0 flex-1 flex-col items-center gap-2.5 overflow-y-auto px-2 py-1 scrollbar-hide">
          {servers.map((server, idx) => {
            const active = !appSettingsOpen && server.id === activeId
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
                  <motion.span
                    layoutId="server-rail-active-indicator"
                    transition={spring.moderate}
                    className="absolute -left-2 h-6 w-1 rounded-r-full"
                    style={{ backgroundColor: color }}
                  />
                )}
                <Tooltip
                  side="right"
                  content={`${server.name} · ${server.health}`}
                  shortcut={idx < 9 ? ['Ctrl', String(idx + 1)] : undefined}
                >
                  <button
                    type="button"
                    onClick={() => onSelect(server.id)}
                    aria-label={server.name}
                    aria-current={active ? 'true' : undefined}
                    className={cn(
                      'group relative flex size-11 items-center justify-center text-[13px] font-semibold transition-[transform,background-color,box-shadow,color] duration-100 active:scale-95',
                      shape.container,
                      active
                        ? 'text-white shadow-[var(--shadow-3)]'
                        : 'bg-card text-foreground/85 shadow-[var(--shadow-1)] hover:bg-surface-4 hover:text-foreground',
                      isDropTarget && 'ring-2 ring-primary',
                    )}
                    style={
                      active
                        ? {
                            backgroundColor: color,
                            boxShadow: `0 0 0 1px ${color}, 0 4px 12px -4px ${color}80`,
                          }
                        : undefined
                    }
                  >
                    <span>{initials(server.name)}</span>
                    <span
                      className={cn(
                        'absolute -right-0.5 -bottom-0.5 size-3 rounded-full border-2 border-rail',
                        HEALTH_DOT[server.health],
                      )}
                    />
                  </button>
                </Tooltip>
              </div>
            )
          })}

          <Tooltip side="right" content="Add Rig Server" shortcut={['Ctrl', 'N']}>
            <button
              type="button"
              onClick={onAdd}
              aria-label="Add Rig server"
              className={cn(
                'mt-0.5 flex size-11 items-center justify-center border border-dashed border-border text-muted-foreground transition-colors duration-80 hover:border-primary/60 hover:bg-hover hover:text-foreground active:scale-95',
                shape.container,
              )}
            >
              <Plus className="size-4.5" strokeWidth={1.75} />
            </button>
          </Tooltip>
        </div>

        <div className="mt-2 flex w-full flex-col items-center gap-1.5 border-t border-sidebar-border/80 px-2 pt-3">
          <Tooltip
            side="right"
            content={theme === 'dark' ? 'Switch to Light Mode' : 'Switch to Dark Mode'}
          >
            <button
              type="button"
              onClick={onToggleTheme}
              aria-label={theme === 'dark' ? 'Switch to light theme' : 'Switch to dark theme'}
              className={cn(
                'flex size-10 items-center justify-center text-muted-foreground transition-colors duration-80 hover:bg-hover hover:text-foreground',
                shape.button,
              )}
            >
              {theme === 'dark' ? (
                <Sun className="size-4.5" strokeWidth={1.65} />
              ) : (
                <Moon className="size-4.5" strokeWidth={1.65} />
              )}
            </button>
          </Tooltip>
          <div className="relative flex w-full items-center justify-center">
            {appSettingsOpen && (
              <motion.span
                layoutId="server-rail-active-indicator"
                transition={spring.moderate}
                className="absolute -left-2 h-6 w-1 rounded-r-full bg-primary"
              />
            )}
            <Tooltip side="right" content="App Settings" shortcut={['Ctrl', ',']}>
              <button
                type="button"
                onClick={onOpenAppSettings}
                aria-label="App Settings"
                aria-current={appSettingsOpen ? 'page' : undefined}
                className={cn(
                  'flex size-10 items-center justify-center transition-colors duration-80',
                  shape.button,
                  appSettingsOpen
                    ? 'bg-active text-primary'
                    : 'text-muted-foreground hover:bg-hover hover:text-foreground',
                )}
              >
                <Settings className="size-4.5" strokeWidth={appSettingsOpen ? 2 : 1.65} />
              </button>
            </Tooltip>
          </div>
        </div>
      </TooltipGroup>
    </aside>
  )
}
