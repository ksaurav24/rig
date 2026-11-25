import { getCurrentWebview } from '@tauri-apps/api/webview'
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from 'react'
import { useShapeContext, type ShapeVariant } from '@rig/ui/lib/shape-context'
import { useSizeContext } from '@rig/ui/lib/size-context'

export type ThemeMode = 'dark' | 'light' | 'system'
export type SurfacePreset = 'graphite' | 'obsidian' | 'oled' | 'slate'
export type AccentPreset = 'emerald' | 'indigo' | 'sky' | 'violet' | 'amber' | 'rose' | 'graphite'
export type UiDensity = 'compact' | 'default' | 'comfortable'
export type SidebarLayout = 'expanded' | 'compact'
export type TimeStyle = 'relative' | 'exact'
export type TimeZoneMode = 'local' | 'utc'
export type DefaultSection = 'overview' | 'apps' | 'local' | 'databases'
export type LogFontSize = 12 | 13 | 14 | 15

export const MIN_ZOOM = 0.8
export const MAX_ZOOM = 1.75
export const ZOOM_STEP = 0.1
export const ZOOM_PRESETS = [0.9, 1.0, 1.1, 1.25, 1.4, 1.5] as const

export function clampZoom(z: number): number {
  const clamped = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, z))
  return Math.round(clamped * 100) / 100
}

export interface DesktopPreferences {
  theme: ThemeMode
  surface: SurfacePreset
  accent: AccentPreset
  density: UiDensity
  shape: ShapeVariant
  zoom: number
  sidebarLayout: SidebarLayout
  showShortcutHints: boolean
  timeStyle: TimeStyle
  timeZone: TimeZoneMode
  defaultSection: DefaultSection
  logFontSize: LogFontSize
  logWrap: boolean
  notificationsEnabled: boolean
}

export const DEFAULT_PREFS: DesktopPreferences = {
  theme: 'dark',
  surface: 'graphite',
  accent: 'emerald',
  density: 'default',
  shape: 'rounded',
  zoom: 1.0,
  sidebarLayout: 'expanded',
  showShortcutHints: true,
  timeStyle: 'relative',
  timeZone: 'local',
  defaultSection: 'overview',
  logFontSize: 13,
  logWrap: false,
  notificationsEnabled: true,
}

export const ACCENT_PRESETS: {
  value: AccentPreset
  label: string
  swatchLight: string
  swatchDark: string
}[] = [
  { value: 'emerald', label: 'Emerald', swatchLight: 'oklch(0.5 0.115 158)', swatchDark: 'oklch(0.74 0.135 158)' },
  { value: 'indigo', label: 'Indigo', swatchLight: 'oklch(0.49 0.165 264)', swatchDark: 'oklch(0.72 0.145 264)' },
  { value: 'sky', label: 'Sky', swatchLight: 'oklch(0.51 0.135 230)', swatchDark: 'oklch(0.75 0.125 225)' },
  { value: 'violet', label: 'Violet', swatchLight: 'oklch(0.50 0.175 295)', swatchDark: 'oklch(0.74 0.155 295)' },
  { value: 'amber', label: 'Amber', swatchLight: 'oklch(0.56 0.145 62)', swatchDark: 'oklch(0.79 0.14 70)' },
  { value: 'rose', label: 'Rose', swatchLight: 'oklch(0.52 0.18 12)', swatchDark: 'oklch(0.72 0.165 14)' },
  { value: 'graphite', label: 'Graphite', swatchLight: 'oklch(0.24 0.008 70)', swatchDark: 'oklch(0.91 0.006 75)' },
]

export const SURFACE_PRESETS: {
  value: SurfacePreset
  label: string
  bg: string
  card: string
}[] = [
  { value: 'graphite', label: 'Warm Graphite', bg: '#181715', card: '#22211e' },
  { value: 'obsidian', label: 'Obsidian', bg: '#0e1015', card: '#181b22' },
  { value: 'oled', label: 'OLED Black', bg: '#040404', card: '#141413' },
  { value: 'slate', label: 'Midnight Slate', bg: '#0c1324', card: '#152039' },
]

const STORAGE_KEY = 'rig-desktop-prefs'

function loadPrefs(): DesktopPreferences {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return DEFAULT_PREFS
    const p = JSON.parse(raw) as Partial<DesktopPreferences> & { radius?: string }

    // Migrate legacy `warm` -> `graphite` and `radius` -> `shape` seamlessly
    const surface: SurfacePreset =
      p.surface === 'graphite' || p.surface === 'obsidian' || p.surface === 'oled' || p.surface === 'slate'
        ? p.surface
        : DEFAULT_PREFS.surface

    let shape: ShapeVariant = DEFAULT_PREFS.shape
    if (p.shape === 'sharp' || p.shape === 'rounded' || p.shape === 'pill') {
      shape = p.shape
    } else if (p.radius === 'sharp') {
      shape = 'sharp'
    } else if (p.radius === 'soft') {
      shape = 'pill'
    }

    const logFontSize: LogFontSize =
      p.logFontSize === 12 || p.logFontSize === 13 || p.logFontSize === 14 || p.logFontSize === 15
        ? p.logFontSize
        : DEFAULT_PREFS.logFontSize

    return {
      theme: p.theme === 'dark' || p.theme === 'light' || p.theme === 'system' ? p.theme : DEFAULT_PREFS.theme,
      surface,
      accent: ACCENT_PRESETS.some((a) => a.value === p.accent)
        ? (p.accent as AccentPreset)
        : DEFAULT_PREFS.accent,
      density:
        p.density === 'compact' || p.density === 'default' || p.density === 'comfortable'
          ? p.density
          : DEFAULT_PREFS.density,
      shape,
      zoom: typeof p.zoom === 'number' && Number.isFinite(p.zoom) ? clampZoom(p.zoom) : DEFAULT_PREFS.zoom,
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
      logFontSize,
      logWrap: typeof p.logWrap === 'boolean' ? p.logWrap : DEFAULT_PREFS.logWrap,
      notificationsEnabled:
        typeof p.notificationsEnabled === 'boolean'
          ? p.notificationsEnabled
          : DEFAULT_PREFS.notificationsEnabled,
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

async function applyWebviewZoom(zoom: number): Promise<void> {
  const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
  if (isTauri) {
    try {
      await getCurrentWebview().setZoom(zoom)
      document.documentElement.style.removeProperty('zoom')
      return
    } catch {
      // Fall back to CSS zoom if native zoom call is unavailable
    }
  }
  if (typeof document !== 'undefined') {
    if (zoom === 1) {
      document.documentElement.style.removeProperty('zoom')
    } else {
      document.documentElement.style.setProperty('zoom', String(zoom))
    }
  }
}

interface PreferencesContextValue {
  prefs: DesktopPreferences
  resolvedTheme: 'dark' | 'light'
  updatePrefs: (patch: Partial<DesktopPreferences>) => void
  zoomIn: () => void
  zoomOut: () => void
  resetZoom: () => void
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

    if (prefs.surface === 'graphite') root.removeAttribute('data-surface')
    else root.setAttribute('data-surface', prefs.surface)

    if (prefs.density === 'default') root.removeAttribute('data-density')
    else root.setAttribute('data-density', prefs.density)
  }, [prefs.accent, prefs.density, prefs.surface, resolvedTheme])

  useEffect(() => {
    void applyWebviewZoom(prefs.zoom)
  }, [prefs.zoom])

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
        const next: DesktopPreferences = {
          ...prev,
          ...patch,
          zoom: patch.zoom !== undefined ? clampZoom(patch.zoom) : prev.zoom,
        }
        persist(next)
        return next
      })
    },
    [persist],
  )

  const zoomIn = useCallback(() => {
    setPrefs((prev) => {
      const next = { ...prev, zoom: clampZoom(prev.zoom + ZOOM_STEP) }
      persist(next)
      return next
    })
  }, [persist])

  const zoomOut = useCallback(() => {
    setPrefs((prev) => {
      const next = { ...prev, zoom: clampZoom(prev.zoom - ZOOM_STEP) }
      persist(next)
      return next
    })
  }, [persist])

  const resetZoom = useCallback(() => {
    updatePrefs({ zoom: 1.0 })
  }, [updatePrefs])

  const resetPrefs = useCallback(() => {
    setPrefs(DEFAULT_PREFS)
    persist(DEFAULT_PREFS)
  }, [persist])

  const value = useMemo<PreferencesContextValue>(
    () => ({ prefs, resolvedTheme, updatePrefs, zoomIn, zoomOut, resetZoom, resetPrefs }),
    [prefs, resetPrefs, resetZoom, resolvedTheme, updatePrefs, zoomIn, zoomOut],
  )

  return <PreferencesContext.Provider value={value}>{children}</PreferencesContext.Provider>
}

export function usePreferences(): PreferencesContextValue {
  const ctx = useContext(PreferencesContext)
  if (!ctx) throw new Error('usePreferences must be used within PreferencesProvider')
  return ctx
}

/** Keeps FluidProvider's ShapeContext and SizeContext synced with stored desktop preferences. */
export function FluidPreferencesSync() {
  const { prefs } = usePreferences()
  const { shape, setShape } = useShapeContext()
  const { size, setSize } = useSizeContext()

  useEffect(() => {
    if (shape !== prefs.shape) setShape(prefs.shape)
  }, [prefs.shape, shape, setShape])

  useEffect(() => {
    const targetSize = prefs.density === 'compact' ? 'compact' : 'default'
    if (size !== targetSize) setSize(targetSize)
  }, [prefs.density, size, setSize])

  return null
}
