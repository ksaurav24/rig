import { getTimePreferences, type HourCycleMode, type TimeZoneMode } from '@/lib/preferences'

const RELATIVE = new Intl.RelativeTimeFormat(undefined, { numeric: 'auto' })

const STEPS: [Intl.RelativeTimeFormatUnit, number][] = [
  ['year', 31_536_000],
  ['month', 2_592_000],
  ['day', 86_400],
  ['hour', 3_600],
  ['minute', 60],
]

function hour12Option(hourCycle: HourCycleMode): boolean | undefined {
  if (hourCycle === '12h') return true
  if (hourCycle === '24h') return false
  return undefined
}

function formatDateTime(
  date: Date,
  opts: {
    dateStyle?: 'short' | 'medium'
    timeStyle?: 'short' | 'medium'
    timeZone: TimeZoneMode
    hourCycle: HourCycleMode
  },
): string {
  const h12 = hour12Option(opts.hourCycle)
  const formatter = new Intl.DateTimeFormat(undefined, {
    ...(opts.dateStyle ? { dateStyle: opts.dateStyle } : {}),
    ...(opts.timeStyle ? { timeStyle: opts.timeStyle } : {}),
    ...(opts.timeZone === 'utc' ? { timeZone: 'UTC' } : {}),
    ...(h12 !== undefined ? { hour12: h12 } : {}),
  })
  const text = formatter.format(date)
  return opts.timeZone === 'utc' ? `${text} UTC` : text
}

/**
 * Formats an ISO timestamp according to user preferences:
 * - Relative mode: "3 minutes ago", "yesterday", "just now"
 * - Exact mode: concise local or UTC date + time
 */
export function timeAgo(iso: string | null | undefined, now = Date.now()): string {
  if (!iso) return 'never'
  const prefs = getTimePreferences()
  const date = new Date(iso)
  if (prefs.timeStyle === 'exact') {
    return formatDateTime(date, {
      dateStyle: 'short',
      timeStyle: 'short',
      timeZone: prefs.timeZone,
      hourCycle: prefs.hourCycle,
    })
  }
  const seconds = Math.round((date.getTime() - now) / 1000)
  for (const [unit, size] of STEPS) {
    if (Math.abs(seconds) >= size) return RELATIVE.format(Math.round(seconds / size), unit)
  }
  return 'just now'
}

export function fullDate(iso: string | null | undefined): string {
  if (!iso) return ''
  const prefs = getTimePreferences()
  return formatDateTime(new Date(iso), {
    dateStyle: 'medium',
    timeStyle: 'short',
    timeZone: prefs.timeZone,
    hourCycle: prefs.hourCycle,
  })
}

/** Compact HH:MM:SS timestamp for log streams, respecting timezone and 12h/24h preferences. */
export function logTime(iso: string): string {
  const prefs = getTimePreferences()
  const h12 = hour12Option(prefs.hourCycle)
  return new Intl.DateTimeFormat(undefined, {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: h12 ?? false,
    ...(prefs.timeZone === 'utc' ? { timeZone: 'UTC' } : {}),
  }).format(new Date(iso))
}

export function bytes(n: number): string {
  if (n < 1024) return `${n} B`
  const units = ['KB', 'MB', 'GB', 'TB']
  let value = n / 1024
  let i = 0
  while (value >= 1024 && i < units.length - 1) {
    value /= 1024
    i++
  }
  return `${value >= 100 ? value.toFixed(0) : value.toFixed(1)} ${units[i]}`
}

export function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean)
  return ((parts[0]?.[0] ?? '') + (parts.length > 1 ? parts.at(-1)![0] : '')).toUpperCase() || '?'
}

/** "42s", "3m 05s". Runs to `now` while `end` is missing. */
export function duration(start: string | null | undefined, end: string | null | undefined, now = Date.now()): string {
  if (!start) return ''
  const seconds = Math.max(0, Math.round(((end ? new Date(end).getTime() : now) - new Date(start).getTime()) / 1000))
  if (seconds < 60) return `${seconds}s`
  return `${Math.floor(seconds / 60)}m ${String(seconds % 60).padStart(2, '0')}s`
}

export function percent(used: number, total: number): number {
  return total > 0 ? Math.min(100, Math.round((used / total) * 100)) : 0
}
