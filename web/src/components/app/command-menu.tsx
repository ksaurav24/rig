import { useNavigate } from '@tanstack/react-router'
import { Search } from 'lucide-react'
import { useMemo, useState } from 'react'
import type { Schemas } from '@/api/client'
import { THEMES } from '@/components/app/theme-toggle'
import {
  CommandMenu as FluidCommandMenu,
  CommandMenuDialog,
  CommandMenuEmpty,
  CommandMenuFooter,
  CommandMenuInput,
  CommandMenuList,
  type CommandMenuItemData,
} from '@/components/fluid/command-menu'
import { Button } from '@/components/ui/button'
import { KeyCaps } from '@/lib/key-cap'
import { navFor } from '@/lib/nav'
import { useTheme } from '@/lib/theme'

const IS_MAC = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform)

export function CommandMenu({ me }: { me: Schemas['Me'] }) {
  const [open, setOpen] = useState(false)
  const navigate = useNavigate()
  const { setTheme } = useTheme()

  const items = useMemo<CommandMenuItemData[]>(() => {
    const navItems: CommandMenuItemData[] = navFor(me.role).flatMap((group) =>
      group.items.map((item) => ({
        value: `nav:${item.to}`,
        label: item.title,
        group: group.label,
        icon: item.icon,
        keywords: item.keywords?.split(' ').filter(Boolean),
        onSelect: () => void navigate({ to: item.to }),
      })),
    )
    const themeItems: CommandMenuItemData[] = THEMES.map(({ value, label, icon }) => ({
      value: `theme:${value}`,
      label,
      group: 'Theme',
      icon,
      keywords: ['theme', 'appearance', value],
      onSelect: () => setTheme(value),
    }))
    return [...navItems, ...themeItems]
  }, [me.role, navigate, setTheme])

  return (
    <>
      <Button
        variant="outline"
        size="sm"
        onClick={() => setOpen(true)}
        className="w-8 justify-center px-0 text-muted-foreground sm:w-52 sm:justify-between sm:px-2.5"
        aria-label="Search and jump to"
      >
        <span className="inline-flex items-center gap-1.5">
          <Search className="size-3.5" />
          <span className="hidden text-[12.5px] sm:inline">Jump to…</span>
        </span>
        <KeyCaps keys={IS_MAC ? '⌘K' : ['Ctrl', 'K']} className="hidden sm:inline-flex" />
      </Button>

      <CommandMenuDialog
        open={open}
        onOpenChange={setOpen}
        shortcut="mod+k"
        title="Jump to"
        description="Search pages and actions"
      >
        <FluidCommandMenu items={items}>
          <CommandMenuInput placeholder="Search pages and actions…" />
          <CommandMenuList />
          <CommandMenuEmpty>Nothing matches that.</CommandMenuEmpty>
          <CommandMenuFooter />
        </FluidCommandMenu>
      </CommandMenuDialog>
    </>
  )
}
