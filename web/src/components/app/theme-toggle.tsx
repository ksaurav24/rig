import { Monitor, Moon, Sun } from 'lucide-react'
import { DropdownContent, DropdownMenu, DropdownTrigger, MenuItem } from '@rig/ui/dropdown'
import { Button } from '@/components/ui/button'
import { useTheme, type Theme } from '@/lib/theme'

export const THEMES = [
  { value: 'light', label: 'Light', icon: Sun },
  { value: 'dark', label: 'Dark', icon: Moon },
  { value: 'system', label: 'Match system', icon: Monitor },
] as const

export function ThemeToggle() {
  const { theme, setTheme } = useTheme()
  const checkedIndex = THEMES.findIndex((t) => t.value === theme)

  return (
    <DropdownMenu>
      <DropdownTrigger render={<Button variant="ghost" size="icon-sm" aria-label="Change theme" />}>
        <Sun className="dark:hidden" />
        <Moon className="hidden dark:block" />
      </DropdownTrigger>
      <DropdownContent align="end" checkedIndex={checkedIndex >= 0 ? checkedIndex : undefined} className="w-44">
        {THEMES.map(({ value, label, icon }, index) => (
          <MenuItem
            key={value}
            index={index}
            icon={icon}
            label={label}
            checked={theme === value}
            onSelect={() => setTheme(value as Theme)}
          />
        ))}
      </DropdownContent>
    </DropdownMenu>
  )
}
