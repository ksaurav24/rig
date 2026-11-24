import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from 'react'

export type ThemeMode = 'dark' | 'light' | 'system'
export type SurfacePreset = 'obsidian' | 'oled' | 'slate' | 'warm'
export type AccentPreset = 'emerald' | 'indigo' | 'sky' | 'violet' | 'amber' | 'rose' | 'graphite'
export type UiDensity = 'compact' | 'default' | 'comfortable'
export type CornerRadius = 'sharp' | 'rounded' | 'soft'
export type SidebarLayout = 'expanded' | 'compact'
export type TimeStyle = 'relative' | 'exact'
export type TimeZoneMode = 'local' | 'utc'
export type DefaultSection = 'overview' | 'apps' | 'local' | 'databases'
export type LogFontSize = 11 | 12 | 13 | 14

export interface DesktopPreferences {
  theme: ThemeMode
  surface: SurfacePreset
  accent: AccentPreset
  density: UiDensity
  radius: CornerRadius
  sidebarLayout: SidebarLayout
  showShortcutHints: boolean
  timeStyle: TimeStyle
  timeZone: TimeZoneMode
  defaultSection: DefaultSection
  logFontSize: LogFontSize
  logWrap: boolean
}

export const DEFAULT_PREFS: DesktopPreferences = {
  theme: 'dark',
  surface: 'obsidian',
  accent: 'emerald',
  density: 'default',
  radius: 'rounded',
  sidebarLayout: 'expanded',
  showShortcutHints: true,
  timeStyle: 'relative',
  timeZone: 'local',
  defaultSection: 'overview',
  logFontSize: 12,
  logWrap: false,
}

export const ACCENT_PRESETS: { value: AccentPreset; label: string; swatch: string }[] = [
  { value: 'emerald', label: 'Emerald', swatch: '#10b981' },
  { value: 'indigo', label: 'Indigo', swatch: '#6366f1' },
  { value: 'sky', label: 'Sky', swatch: '#0ea5e9' },
  { value: 'violet', label: 'Violet', swatch: '#8b5cf6' },
  { value: 'amber', label: 'Amber', swatch: '#f59e0b' },
  { value: 'rose', label: 'Rose', swatch: '#f43f5e' },
  { value: 'graphite', label: 'Graphite', swatch: '#94a3b8' },
]

export const SURFACE_PRESETS: {
  value: SurfacePreset
  label: string
  bg: string
  surface: string
}[] = [
  { value: 'obsidian', label: 'Obsidian', bg: '#0c0d10', surface: '#13151a' },
  { value: 'oled', label: 'OLED Black', bg: '#000000', surface: '#0a0b0e' },
  { value: 'slate', label: 'Midnight Slate', bg: '#0b1120', surface: '#111a2e' },
  { value: 'warm', label: 'Warm Charcoal', bg: '#111110', surface: '#191816' },
]

const STORAGE_KEY = 'rig-desktop-prefs'

function loadPrefs(): DesktopPreferences {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return DEFAULT_PREFS
    const p = JSON.parse(raw) as Partial<DesktopPreferences>
    return {
      theme: p.theme === 'dark' || p.theme === 'light' || p.theme === 'system' ? p.theme : DEFAULT_PREFS.theme,
      surface:
        p.surface === 'obsidian' || p.surface === 'oled' || p.surface === 'slate' || p.surface === 'warm'
          ? p.surface
          : DEFAULT_PREFS.surface,
      accent: ACCENT_PRESETS.some((a) => a.value === p.accent)
        ? (p.accent as AccentPreset)
        : DEFAULT_PREFS.accent,
      density:
        p.density === 'compact' || p.density === 'default' || p.density === 'comfortable'
          ? p.density
          : DEFAULT_PREFS.density,
      radius:
        p.radius === 'sharp' || p.radius === 'rounded' || p.radius === 'soft'
          ? p.radius
          : DEFAULT_PREFS.radius,
      sidebarLayout:
        p.sidebarLayout === 'expanded' || p.sidebarLayout === 'compact'
          ? p.sidebarLayout
          : DEFAULT_PREFS.sidebarLayout,
      showShortcutHints:
        typeof p.showShortcutHints === 'boolean'
          ? p.showShortcutHints
          : DEFAULT_PREFS.showShortcutHints,
      timeStyle: p.timeStyle === 'relative' || p.timeStyle === 'exact' ? p.timeStyle : DEFAULT_PREFS.timeStyle,
      timeZone: p.timeZone === 'local' || p.timeZone === 'utc' ? p.timeZone : DEFAULT_PREFS.timeZone,
      defaultSection:
        p.defaultSection === 'overview' ||
        p.defaultSection === 'apps' ||
        p.defaultSection === 'local' ||
        p.defaultSection === 'databases'
          ? p.defaultSection
          : DEFAULT_PREFS.defaultSection,
      logFontSize:
        p.logFontSize === 11 || p.logFontSize === 12 || p.logFontSize === 13 || p.logFontSize === 14
          ? p.logFontSize
          : DEFAULT_PREFS.logFontSize,
      logWrap: typeof p.logWrap === 'boolean' ? p.logWrap : DEFAULT_PREFS.logWrap,
    }
  } catch {
    return DEFAULT_PREFS
  }
}

let liveSnapshot: DesktopPreferences = typeof window !== 'undefined' ? loadPrefs() : DEFAULT_PREFS

export function getTimePreferences(): Pick<DesktopPreferences, 'timeStyle' | 'timeZone'> {
  return {
    timeStyle: liveSnapshot.timeStyle,
    timeZone: liveSnapshot.timeZone,
  }
}

function getSystemDark(): boolean {
  if (typeof window === 'undefined' || !window.matchMedia) return true
  return window.matchMedia('(prefers-color-scheme: dark)').matches
}

interface PreferencesContextValue {
  prefs: DesktopPreferences
  resolvedTheme: 'dark' | 'light'
  updatePrefs: (patch: Partial<DesktopPreferences>) => void
  resetPrefs: () => void
}

const PreferencesContext = createContext<PreferencesContextValue | null>(null)

export function PreferencesProvider({ children }: { children: ReactNode }) {
  const [prefs, setPrefs] = useState<DesktopPreferences>(loadPrefs)
  const [systemDark, setSystemDark] = useState<boolean>(getSystemDark)

  liveSnapshot = prefs

  useEffect(() => {
    if (typeof window === 'undefined' || !window.matchMedia) return
    const mq = window.matchMedia('(prefers-color-scheme: dark)')
    const onChange = (e: MediaQueryListEvent) => setSystemDark(e.matches)
    mq.addEventListener('change', onChange)
    return () => mq.removeEventListener('change', onChange)
  }, [])

  const resolvedTheme: 'dark' | 'light' =
    prefs.theme === 'system' ? (systemDark ? 'dark' : 'light') : prefs.theme

  useEffect(() => {
    const root = document.documentElement
    root.classList.toggle('light', resolvedTheme === 'light')
    root.classList.toggle('dark', resolvedTheme === 'dark')

    if (prefs.accent === 'emerald') root.removeAttribute('data-accent')
    else root.setAttribute('data-accent', prefs.accent)

    if (prefs.surface === 'obsidian') root.removeAttribute('data-surface')
    else root.setAttribute('data-surface', prefs.surface)

    if (prefs.density === 'default') root.removeAttribute('data-density')
    else root.setAttribute('data-density', prefs.density)

    if (prefs.radius === 'rounded') root.removeAttribute('data-radius')
    else root.setAttribute('data-radius', prefs.radius)
  }, [prefs.accent, prefs.density, prefs.radius, prefs.surface, resolvedTheme])

  const persist = useCallback((next: DesktopPreferences) => {
    liveSnapshot = next
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(next))
    } catch {
      // ignore storage quota errors
    }
  }, [])

  const updatePrefs = useCallback(
    (patch: Partial<DesktopPreferences>) => {
      setPrefs((prev) => {
        const next = { ...prev, ...patch }
        persist(next)
        return next
      })
    },
    [persist],
  )

  const resetPrefs = useCallback(() => {
    setPrefs(DEFAULT_PREFS)
    persist(DEFAULT_PREFS)
  }, [persist])

  const value = useMemo<PreferencesContextValue>(
    () => ({ prefs, resolvedTheme, updatePrefs, resetPrefs }),
    [prefs, resetPrefs, resolvedTheme, updatePrefs],
  )

  return <PreferencesContext.Provider value={value}>{children}</PreferencesContext.Provider>
}

export function usePreferences(): PreferencesContextValue {
  const ctx = useContext(PreferencesContext)
  if (!ctx) throw new Error('usePreferences must be used within PreferencesProvider')
  return ctx
}
