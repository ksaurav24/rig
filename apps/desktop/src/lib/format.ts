import { getTimePreferences } from './preferences'

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1)
  const val = bytes / 1024 ** i
  return `${val >= 100 || i === 0 ? Math.round(val) : val.toFixed(1)} ${units[i]}`
}

export function formatPercent(used: number, total: number): number {
  if (!total || total <= 0) return 0
  return Math.min(100, Math.max(0, Math.round((used / total) * 100)))
}

export function formatExactTime(iso: string): string {
  const t = Date.parse(iso)
  if (Number.isNaN(t)) return iso
  const { timeZone } = getTimePreferences()
  const d = new Date(t)
  return new Intl.DateTimeFormat(undefined, {
    month: 'short',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
    timeZone: timeZone === 'utc' ? 'UTC' : undefined,
  }).format(d)
}

export function relativeTime(iso?: string | null): string {
  if (!iso) return 'never'
  const t = Date.parse(iso)
  if (Number.isNaN(t)) return iso
  const { timeStyle } = getTimePreferences()
  if (timeStyle === 'exact') {
    return formatExactTime(iso)
  }
  const diffSec = Math.max(0, Math.floor((Date.now() - t) / 1000))
  if (diffSec < 10) return 'just now'
  if (diffSec < 60) return `${diffSec}s ago`
  const mins = Math.floor(diffSec / 60)
  if (mins < 60) return `${mins}m ago`
  const hrs = Math.floor(mins / 60)
  if (hrs < 24) return `${hrs}h ago`
  return `${Math.floor(hrs / 24)}d ago`
}

export function formatLogTime(iso: string): string {
  const t = Date.parse(iso)
  if (Number.isNaN(t)) return iso.slice(11, 19) || '--:--:--'
  const { timeZone } = getTimePreferences()
  const d = new Date(t)
  if (timeZone === 'utc') {
    return d.toISOString().slice(11, 19)
  }
  return d.toTimeString().slice(0, 8)
}

export function initials(name: string): string {
  const parts = name
    .trim()
    .split(/[\s._-]+/)
    .filter(Boolean)
  if (parts.length === 0) return 'R'
  if (parts.length === 1) return parts[0].slice(0, 2).toUpperCase()
  return (parts[0][0] + parts[1][0]).toUpperCase()
}

export const ACCENT_COLORS: Record<string, string> = {
  emerald: '#10b981',
  sky: '#0ea5e9',
  violet: '#8b5cf6',
  amber: '#f59e0b',
  rose: '#f43f5e',
  cyan: '#06b6d4',
  indigo: '#6366f1',
  teal: '#14b8a6',
}

export function accentHex(name?: string | null): string {
  if (!name) return '#10b981'
  if (name.startsWith('#')) return name
  return ACCENT_COLORS[name] ?? '#10b981'
}
