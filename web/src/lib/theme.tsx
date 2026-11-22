import { createContext, use, useCallback, useEffect, useMemo, useState, type ReactNode } from 'react'

export type Theme = 'system' | 'light' | 'dark'

const KEY = 'rig-theme'
const DARK_QUERY = '(prefers-color-scheme: dark)'

type ThemeContext = {
  /** What the person chose. */
  theme: Theme
  /** What is on screen. */
  resolved: 'light' | 'dark'
  setTheme: (theme: Theme) => void
}

const Context = createContext<ThemeContext | null>(null)

function isTheme(value: unknown): value is Theme {
  return value === 'system' || value === 'light' || value === 'dark'
}

function stored(): Theme {
  try {
    const value = localStorage.getItem(KEY)
    return isTheme(value) ? value : 'system'
  } catch {
    return 'system'
  }
}

/** Swaps the class without letting every element animate its colours. */
function paint(dark: boolean) {
  const root = document.documentElement
  if (root.classList.contains('dark') === dark) return
  const style = document.createElement('style')
  style.textContent = '*,*::before,*::after{transition:none!important}'
  document.head.appendChild(style)
  root.classList.toggle('dark', dark)
  // Force a style flush so the rule applies to this change only.
  void getComputedStyle(root).opacity
  style.remove()
}

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [theme, setThemeState] = useState<Theme>(stored)
  const [systemDark, setSystemDark] = useState(() => matchMedia(DARK_QUERY).matches)

  useEffect(() => {
    const query = matchMedia(DARK_QUERY)
    const onChange = () => setSystemDark(query.matches)
    query.addEventListener('change', onChange)
    // Keep other tabs in step.
    const onStorage = (event: StorageEvent) => {
      if (event.key === KEY) setThemeState(stored())
    }
    window.addEventListener('storage', onStorage)
    return () => {
      query.removeEventListener('change', onChange)
      window.removeEventListener('storage', onStorage)
    }
  }, [])

  const resolved = theme === 'system' ? (systemDark ? 'dark' : 'light') : theme

  useEffect(() => {
    paint(resolved === 'dark')
  }, [resolved])

  const setTheme = useCallback((next: Theme) => {
    setThemeState(next)
    try {
      localStorage.setItem(KEY, next)
    } catch {
      // Private mode: the choice lasts for this visit only.
    }
  }, [])

  const value = useMemo(() => ({ theme, resolved, setTheme }), [theme, resolved, setTheme])
  return <Context value={value}>{children}</Context>
}

export function useTheme(): ThemeContext {
  const context = use(Context)
  if (!context) throw new Error('useTheme must be used inside ThemeProvider')
  return context
}
