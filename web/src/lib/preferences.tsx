import { createContext, use, useCallback, useEffect, useMemo, useState, type ReactNode } from 'react'
import { useShapeContext, type ShapeVariant } from '@rig/ui/lib/shape-context'
import { useSizeContext, type SizeVariant } from '@rig/ui/lib/size-context'

export type AccentPreset = 'emerald' | 'indigo' | 'sky' | 'violet' | 'amber' | 'rose' | 'graphite'
export type AccentChoice = 'org' | AccentPreset
export type SidebarStyleVariant = 'sidebar' | 'floating' | 'inset'
export type SidebarPeekMode = 'none' | 'hover'
export type TimeStyle = 'relative' | 'exact'
export type TimeZoneMode = 'local' | 'utc'
export type HourCycleMode = 'auto' | '12h' | '24h'
export type DefaultHomeRoute = '/' | '/projects' | '/databases'
export type LogFontSize = 12 | 13 | 14
export type EnvBadgeColor = 'emerald' | 'amber' | 'red' | 'blue' | 'purple' | 'gray'

export const ACCENT_PRESETS: { value: AccentPreset; label: string; swatchLight: string; swatchDark: string }[] = [
  { value: 'emerald', label: 'Emerald', swatchLight: 'oklch(0.5 0.115 158)', swatchDark: 'oklch(0.74 0.135 158)' },
  { value: 'indigo', label: 'Indigo', swatchLight: 'oklch(0.49 0.165 264)', swatchDark: 'oklch(0.72 0.145 264)' },
  { value: 'sky', label: 'Sky', swatchLight: 'oklch(0.51 0.135 230)', swatchDark: 'oklch(0.75 0.125 225)' },
  { value: 'violet', label: 'Violet', swatchLight: 'oklch(0.50 0.175 295)', swatchDark: 'oklch(0.74 0.155 295)' },
  { value: 'amber', label: 'Amber', swatchLight: 'oklch(0.56 0.145 62)', swatchDark: 'oklch(0.79 0.14 70)' },
  { value: 'rose', label: 'Rose', swatchLight: 'oklch(0.52 0.18 12)', swatchDark: 'oklch(0.72 0.165 14)' },
  { value: 'graphite', label: 'Graphite', swatchLight: 'oklch(0.24 0.008 70)', swatchDark: 'oklch(0.91 0.006 75)' },
]

export const ENV_COLORS: { value: EnvBadgeColor; label: string; dot: string }[] = [
  { value: 'emerald', label: 'Emerald', dot: 'bg-emerald-500' },
  { value: 'amber', label: 'Amber', dot: 'bg-amber-500' },
  { value: 'red', label: 'Red', dot: 'bg-red-500' },
  { value: 'blue', label: 'Blue', dot: 'bg-blue-500' },
  { value: 'purple', label: 'Purple', dot: 'bg-purple-500' },
  { value: 'gray', label: 'Gray', dot: 'bg-zinc-400' },
]

export interface UserPreferences {
  accent: AccentChoice
  density: SizeVariant
  shape: ShapeVariant
  sidebarVariant: SidebarStyleVariant
  sidebarPeek: SidebarPeekMode
  timeStyle: TimeStyle
  timeZone: TimeZoneMode
  hourCycle: HourCycleMode
  defaultHome: DefaultHomeRoute
  pinnedProjects: string[]
  logFontSize: LogFontSize
  logWrap: boolean
}

export const DEFAULT_PREFS: UserPreferences = {
  accent: 'org',
  density: 'default',
  shape: 'rounded',
  sidebarVariant: 'sidebar',
  sidebarPeek: 'none',
  timeStyle: 'relative',
  timeZone: 'local',
  hourCycle: 'auto',
  defaultHome: '/',
  pinnedProjects: [],
  logFontSize: 12,
  logWrap: true,
}

const STORAGE_KEY = 'rig-prefs'
const RESOLVED_ACCENT_KEY = 'rig-accent-resolved'

const VALID_ACCENTS = new Set<string>(ACCENT_PRESETS.map((a) => a.value))

export function isAccentPreset(v: unknown): v is AccentPreset {
  return typeof v === 'string' && VALID_ACCENTS.has(v)
}

export function isEnvBadgeColor(v: unknown): v is EnvBadgeColor {
  return typeof v === 'string' && ENV_COLORS.some((c) => c.value === v)
}

function parsePrefs(): UserPreferences {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return DEFAULT_PREFS
    const p = JSON.parse(raw) as Partial<UserPreferences>
    return {
      accent: p.accent === 'org' || isAccentPreset(p.accent) ? p.accent : DEFAULT_PREFS.accent,
      density: p.density === 'compact' || p.density === 'default' ? p.density : DEFAULT_PREFS.density,
      shape: p.shape === 'rounded' || p.shape === 'sharp' || p.shape === 'pill' ? p.shape : DEFAULT_PREFS.shape,
      sidebarVariant:
        p.sidebarVariant === 'sidebar' || p.sidebarVariant === 'floating' || p.sidebarVariant === 'inset'
          ? p.sidebarVariant
          : DEFAULT_PREFS.sidebarVariant,
      sidebarPeek: p.sidebarPeek === 'hover' || p.sidebarPeek === 'none' ? p.sidebarPeek : DEFAULT_PREFS.sidebarPeek,
      timeStyle: p.timeStyle === 'exact' || p.timeStyle === 'relative' ? p.timeStyle : DEFAULT_PREFS.timeStyle,
      timeZone: p.timeZone === 'utc' || p.timeZone === 'local' ? p.timeZone : DEFAULT_PREFS.timeZone,
      hourCycle:
        p.hourCycle === '12h' || p.hourCycle === '24h' || p.hourCycle === 'auto'
          ? p.hourCycle
          : DEFAULT_PREFS.hourCycle,
      defaultHome:
        p.defaultHome === '/' || p.defaultHome === '/projects' || p.defaultHome === '/databases'
          ? p.defaultHome
          : DEFAULT_PREFS.defaultHome,
      pinnedProjects: Array.isArray(p.pinnedProjects)
        ? p.pinnedProjects.filter((id): id is string => typeof id === 'string')
        : DEFAULT_PREFS.pinnedProjects,
      logFontSize: p.logFontSize === 12 || p.logFontSize === 13 || p.logFontSize === 14 ? p.logFontSize : DEFAULT_PREFS.logFontSize,
      logWrap: typeof p.logWrap === 'boolean' ? p.logWrap : DEFAULT_PREFS.logWrap,
    }
  } catch {
    return DEFAULT_PREFS
  }
}

// Module-level snapshot so non-React helpers (like format.ts) always see live time preferences.
let liveSnapshot: UserPreferences = typeof window !== 'undefined' ? parsePrefs() : DEFAULT_PREFS

export function getTimePreferences(): Pick<UserPreferences, 'timeStyle' | 'timeZone' | 'hourCycle'> {
  return {
    timeStyle: liveSnapshot.timeStyle,
    timeZone: liveSnapshot.timeZone,
    hourCycle: liveSnapshot.hourCycle,
  }
}

export function getDefaultHome(): DefaultHomeRoute {
  return liveSnapshot.defaultHome
}

function applyAccent(accent: AccentPreset) {
  const root = document.documentElement
  if (accent === 'emerald') {
    root.removeAttribute('data-accent')
  } else {
    root.setAttribute('data-accent', accent)
  }
  try {
    localStorage.setItem(RESOLVED_ACCENT_KEY, accent)
  } catch {
    // Ignore private browsing quota errors.
  }
}

interface PreferencesContextValue {
  prefs: UserPreferences
  orgAccent: AccentPreset
  effectiveAccent: AccentPreset
  setOrgAccent: (accent: AccentPreset) => void
  updatePrefs: (patch: Partial<UserPreferences>) => void
  togglePinnedProject: (projectId: string) => void
  isProjectPinned: (projectId: string) => boolean
  resetPrefs: () => void
}

const Context = createContext<PreferencesContextValue | null>(null)

export function PreferencesProvider({ children }: { children: ReactNode }) {
  const [prefs, setPrefs] = useState<UserPreferences>(parsePrefs)
  const [orgAccent, setOrgAccentState] = useState<AccentPreset>('emerald')

  liveSnapshot = prefs

  const persist = useCallback((next: UserPreferences) => {
    liveSnapshot = next
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(next))
    } catch {
      // Ignore private browsing errors.
    }
  }, [])

  const updatePrefs = useCallback(
    (patch: Partial<UserPreferences>) => {
      setPrefs((prev) => {
        const next = { ...prev, ...patch }
        persist(next)
        return next
      })
    },
    [persist],
  )

  const togglePinnedProject = useCallback(
    (projectId: string) => {
      setPrefs((prev) => {
        const exists = prev.pinnedProjects.includes(projectId)
        const pinnedProjects = exists
          ? prev.pinnedProjects.filter((id) => id !== projectId)
          : [...prev.pinnedProjects, projectId]
        const next = { ...prev, pinnedProjects }
        persist(next)
        return next
      })
    },
    [persist],
  )

  const isProjectPinned = useCallback(
    (projectId: string) => prefs.pinnedProjects.includes(projectId),
    [prefs.pinnedProjects],
  )

  const resetPrefs = useCallback(() => {
    setPrefs(DEFAULT_PREFS)
    persist(DEFAULT_PREFS)
  }, [persist])

  const setOrgAccent = useCallback((accent: AccentPreset) => {
    setOrgAccentState((prev) => (prev === accent ? prev : accent))
  }, [])

  useEffect(() => {
    const onStorage = (event: StorageEvent) => {
      if (event.key === STORAGE_KEY) {
        const next = parsePrefs()
        liveSnapshot = next
        setPrefs(next)
      }
    }
    window.addEventListener('storage', onStorage)
    return () => window.removeEventListener('storage', onStorage)
  }, [])

  const effectiveAccent: AccentPreset = prefs.accent === 'org' ? orgAccent : prefs.accent

  useEffect(() => {
    applyAccent(effectiveAccent)
  }, [effectiveAccent])

  const value = useMemo<PreferencesContextValue>(
    () => ({
      prefs,
      orgAccent,
      effectiveAccent,
      setOrgAccent,
      updatePrefs,
      togglePinnedProject,
      isProjectPinned,
      resetPrefs,
    }),
    [prefs, orgAccent, effectiveAccent, setOrgAccent, updatePrefs, togglePinnedProject, isProjectPinned, resetPrefs],
  )

  return <Context value={value}>{children}</Context>
}

export function usePreferences(): PreferencesContextValue {
  const ctx = use(Context)
  if (!ctx) throw new Error('usePreferences must be used inside PreferencesProvider')
  return ctx
}

/** Keeps FluidProvider's ShapeContext and SizeContext synced with stored preferences. */
export function FluidPreferencesSync() {
  const { prefs } = usePreferences()
  const { shape, setShape } = useShapeContext()
  const { size, setSize } = useSizeContext()

  useEffect(() => {
    if (shape !== prefs.shape) setShape(prefs.shape)
  }, [prefs.shape, shape, setShape])

  useEffect(() => {
    if (size !== prefs.density) setSize(prefs.density)
  }, [prefs.density, size, setSize])

  return null
}
