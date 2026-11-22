import { useNavigate } from '@tanstack/react-router'
import { Search } from 'lucide-react'
import { useTheme } from '@/lib/theme'
import { useEffect, useState } from 'react'
import type { Schemas } from '@/api/client'
import { THEMES } from '@/components/app/theme-toggle'
import { Button } from '@/components/ui/button'
import {
  Command,
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from '@/components/ui/command'
import { Kbd } from '@/components/ui/kbd'
import { navFor } from '@/lib/nav'

const IS_MAC = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform)

export function CommandMenu({ me }: { me: Schemas['Me'] }) {
  const [open, setOpen] = useState(false)
  const navigate = useNavigate()
  const { setTheme } = useTheme()

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key.toLowerCase() === 'k' && (e.metaKey || e.ctrlKey)) {
        e.preventDefault()
        setOpen((v) => !v)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  function run(action: () => void) {
    setOpen(false)
    action()
  }

  return (
    <>
      <Button
        variant="outline"
        size="sm"
        onClick={() => setOpen(true)}
        className="w-9 justify-center gap-2 px-0 text-muted-foreground sm:w-56 sm:justify-start sm:px-2.5"
        aria-label="Search and jump to"
      >
        <Search />
        <span className="hidden sm:inline">Jump to…</span>
        <Kbd className="ml-auto hidden sm:inline-flex">{IS_MAC ? '⌘K' : 'Ctrl K'}</Kbd>
      </Button>
      <CommandDialog open={open} onOpenChange={setOpen} title="Jump to" description="Search pages and actions">
        <Command>
          <CommandInput placeholder="Search pages and actions" />
          <CommandList>
            <CommandEmpty>Nothing matches that.</CommandEmpty>
            {navFor(me.role).map((group) => (
              <CommandGroup key={group.label} heading={group.label}>
                {group.items.map((item) => (
                  <CommandItem
                    key={item.to}
                    value={`${item.title} ${item.keywords ?? ''}`}
                    onSelect={() => run(() => void navigate({ to: item.to }))}
                  >
                    <item.icon />
                    {item.title}
                  </CommandItem>
                ))}
              </CommandGroup>
            ))}
            <CommandGroup heading="Theme">
              {THEMES.map(({ value, label, icon: Icon }) => (
                <CommandItem key={value} value={`theme ${label}`} onSelect={() => run(() => setTheme(value))}>
                  <Icon />
                  {label}
                </CommandItem>
              ))}
            </CommandGroup>
          </CommandList>
        </Command>
      </CommandDialog>
    </>
  )
}
