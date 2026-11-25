import { useQuery } from '@tanstack/react-query'
import type { ReactNode } from 'react'
import { Logo } from '@rig/ui/app/logo'
import { desktopApi } from '@/lib/ipc'
import { cn } from '@rig/ui/lib/utils'

export interface AvatarPreset {
  id: string
  label: string
  render: (className?: string) => ReactNode
}

export const AVATAR_PRESETS: AvatarPreset[] = [
  {
    id: 'preset:emerald-orbit',
    label: 'Emerald Orbit',
    render: (className) => (
      <svg viewBox="0 0 64 64" className={className} aria-hidden="true">
        <rect width="64" height="64" fill="#13251e" />
        <circle cx="32" cy="32" r="22" fill="none" stroke="#34d399" strokeWidth="2.5" strokeOpacity="0.45" />
        <circle cx="32" cy="32" r="11" fill="#10b981" />
        <circle cx="48" cy="18" r="5" fill="#a7f3d0" />
      </svg>
    ),
  },
  {
    id: 'preset:warm-horizon',
    label: 'Warm Horizon',
    render: (className) => (
      <svg viewBox="0 0 64 64" className={className} aria-hidden="true">
        <rect width="64" height="64" fill="#271c15" />
        <circle cx="32" cy="28" r="14" fill="#f59e0b" />
        <path d="M0 44 Q18 34 32 42 T64 40 L64 64 L0 64 Z" fill="#b45309" />
        <path d="M0 52 Q24 44 42 52 T64 50 L64 64 L0 64 Z" fill="#78350f" />
      </svg>
    ),
  },
  {
    id: 'preset:indigo-prism',
    label: 'Indigo Prism',
    render: (className) => (
      <svg viewBox="0 0 64 64" className={className} aria-hidden="true">
        <rect width="64" height="64" fill="#171930" />
        <polygon points="32,10 54,48 10,48" fill="#6366f1" fillOpacity="0.88" />
        <polygon points="32,10 54,48 32,48" fill="#a5b4fc" fillOpacity="0.45" />
        <circle cx="32" cy="36" r="5" fill="#eef2ff" />
      </svg>
    ),
  },
  {
    id: 'preset:bauhaus-split',
    label: 'Bauhaus Split',
    render: (className) => (
      <svg viewBox="0 0 64 64" className={className} aria-hidden="true">
        <rect width="64" height="64" fill="#f4efe6" />
        <rect width="32" height="32" fill="#1f1d1a" />
        <circle cx="48" cy="16" r="12" fill="#e11d48" />
        <path d="M0 64 A32 32 0 0 1 32 32 L32 64 Z" fill="#0d9488" />
        <rect x="38" y="38" width="18" height="18" rx="3" fill="#f59e0b" />
      </svg>
    ),
  },
  {
    id: 'preset:sky-signal',
    label: 'Sky Signal',
    render: (className) => (
      <svg viewBox="0 0 64 64" className={className} aria-hidden="true">
        <rect width="64" height="64" fill="#0f2233" />
        <path d="M12 48 A28 28 0 0 1 48 12" fill="none" stroke="#38bdf8" strokeWidth="4" strokeLinecap="round" />
        <path d="M22 48 A18 18 0 0 1 48 22" fill="none" stroke="#7dd3fc" strokeWidth="4" strokeLinecap="round" />
        <circle cx="46" cy="46" r="7" fill="#e0f2fe" />
      </svg>
    ),
  },
  {
    id: 'preset:rose-bloom',
    label: 'Rose Bloom',
    render: (className) => (
      <svg viewBox="0 0 64 64" className={className} aria-hidden="true">
        <rect width="64" height="64" fill="#2b1420" />
        <circle cx="24" cy="24" r="14" fill="#fb7185" fillOpacity="0.85" />
        <circle cx="40" cy="24" r="14" fill="#f43f5e" fillOpacity="0.7" />
        <circle cx="32" cy="40" r="14" fill="#fda4af" fillOpacity="0.8" />
      </svg>
    ),
  },
  {
    id: 'preset:violet-ring',
    label: 'Violet Ring',
    render: (className) => (
      <svg viewBox="0 0 64 64" className={className} aria-hidden="true">
        <rect width="64" height="64" fill="#1e1530" />
        <circle cx="32" cy="32" r="20" fill="none" stroke="#a78bfa" strokeWidth="8" />
        <circle cx="32" cy="32" r="7" fill="#f5f3ff" />
      </svg>
    ),
  },
  {
    id: 'preset:graphite-grid',
    label: 'Graphite Grid',
    render: (className) => (
      <svg viewBox="0 0 64 64" className={className} aria-hidden="true">
        <rect width="64" height="64" fill="#1c1b19" />
        <rect x="10" y="10" width="18" height="18" rx="4" fill="#e7e5e4" />
        <rect x="36" y="10" width="18" height="18" rx="9" fill="#78716c" />
        <rect x="10" y="36" width="18" height="18" rx="9" fill="#57534e" />
        <rect x="36" y="36" width="18" height="18" rx="4" fill="#a8a29e" />
      </svg>
    ),
  },
  {
    id: 'preset:teal-waves',
    label: 'Teal Waves',
    render: (className) => (
      <svg viewBox="0 0 64 64" className={className} aria-hidden="true">
        <rect width="64" height="64" fill="#0d2629" />
        <path d="M0 24 Q16 14 32 24 T64 24 L64 64 L0 64 Z" fill="#14b8a6" fillOpacity="0.65" />
        <path d="M0 38 Q16 28 32 38 T64 38 L64 64 L0 64 Z" fill="#2dd4bf" fillOpacity="0.8" />
        <path d="M0 52 Q16 44 32 52 T64 52 L64 64 L0 64 Z" fill="#99f6e4" />
      </svg>
    ),
  },
  {
    id: 'preset:terminal-spark',
    label: 'Terminal',
    render: (className) => (
      <svg viewBox="0 0 64 64" className={className} aria-hidden="true">
        <rect width="64" height="64" fill="#141816" />
        <path d="M16 22 L28 32 L16 42" fill="none" stroke="#4ade80" strokeWidth="5" strokeLinecap="round" strokeLinejoin="round" />
        <line x1="33" y1="42" x2="48" y2="42" stroke="#bbf7d0" strokeWidth="5" strokeLinecap="round" />
      </svg>
    ),
  },
  {
    id: 'preset:terracotta-arch',
    label: 'Terracotta Arch',
    render: (className) => (
      <svg viewBox="0 0 64 64" className={className} aria-hidden="true">
        <rect width="64" height="64" fill="#2c1b14" />
        <path d="M14 56 V32 A18 18 0 0 1 50 32 V56 Z" fill="#ea580c" />
        <path d="M23 56 V34 A9 9 0 0 1 41 34 V56 Z" fill="#fed7aa" />
      </svg>
    ),
  },
  {
    id: 'preset:cobalt-comet',
    label: 'Cobalt Comet',
    render: (className) => (
      <svg viewBox="0 0 64 64" className={className} aria-hidden="true">
        <rect width="64" height="64" fill="#111b30" />
        <path d="M10 54 L42 22" stroke="#60a5fa" strokeWidth="7" strokeLinecap="round" strokeOpacity="0.55" />
        <path d="M20 44 L44 20" stroke="#93c5fd" strokeWidth="4" strokeLinecap="round" />
        <circle cx="45" cy="19" r="9" fill="#eff6ff" />
      </svg>
    ),
  },
]

const PRESET_MAP = new Map(AVATAR_PRESETS.map((p) => [p.id, p]))

export function nameInitials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean)
  if (parts.length === 0) return '?'
  if (parts.length === 1) return parts[0]!.slice(0, 2).toUpperCase()
  return `${parts[0]![0] ?? ''}${parts[parts.length - 1]![0] ?? ''}`.toUpperCase()
}

/**
 * Resolves a Rig media path (`/api/v1/media/{id}`) through the Rust Tauri IPC
 * bridge (honoring bearer auth + pinned TLS + strict webview CSP) and caches
 * the resulting `data:` URL in React Query.
 */
export function useServerMediaUrl(serverId: string | null | undefined, url?: string | null) {
  const isMedia = Boolean(serverId && url && url.startsWith('/api/v1/media/'))
  const query = useQuery({
    queryKey: ['desktop', serverId, 'media', url],
    queryFn: () => desktopApi.fetchMediaDataUrl(serverId!, url!),
    enabled: isMedia,
    staleTime: 1000 * 60 * 30,
    gcTime: 1000 * 60 * 60,
    retry: 1,
  })
  if (!url) return null
  if (url.startsWith('preset:')) return null
  if (isMedia) return query.data ?? null
  return url
}

export function UserAvatar({
  serverId,
  name,
  avatarUrl,
  className,
}: {
  serverId?: string | null
  name: string
  avatarUrl?: string | null
  className?: string
}) {
  const preset = avatarUrl ? PRESET_MAP.get(avatarUrl) : undefined
  const resolved = useServerMediaUrl(serverId, avatarUrl)

  if (preset) {
    return (
      <span
        className={cn(
          'inline-flex shrink-0 overflow-hidden rounded-lg border border-border/70 bg-surface',
          className,
        )}
      >
        {preset.render('size-full block')}
      </span>
    )
  }

  if (resolved) {
    return (
      <img
        src={resolved}
        alt={name}
        referrerPolicy="no-referrer"
        className={cn(
          'shrink-0 rounded-lg border border-border/70 bg-surface object-cover',
          className,
        )}
      />
    )
  }

  return (
    <span
      className={cn(
        'inline-flex shrink-0 items-center justify-center rounded-lg border border-border/70 bg-accent font-semibold text-foreground select-none',
        className,
      )}
    >
      {nameInitials(name)}
    </span>
  )
}

export function ServerOrgMark({
  serverId,
  logoUrl,
  name,
  className = 'size-8',
}: {
  serverId?: string | null
  logoUrl?: string | null
  name?: string
  className?: string
}) {
  const resolved = useServerMediaUrl(serverId, logoUrl)
  if (resolved) {
    return (
      <img
        src={resolved}
        alt={name ? `${name} logo` : 'Organization logo'}
        referrerPolicy="no-referrer"
        className={cn(
          'shrink-0 rounded-lg border border-border/70 bg-surface object-cover',
          className,
        )}
      />
    )
  }
  return <Logo className={className} />
}
