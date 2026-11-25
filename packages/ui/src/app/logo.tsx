import { useEffect, useState } from 'react'
import { cn } from '../lib/utils'

export function Logo({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 32 32" aria-hidden="true" className={cn('size-6', className)}>
      <rect width="32" height="32" rx="7" className="fill-primary" />
      <path
        d="M9 23V9h8.2a4.6 4.6 0 0 1 1.7 8.9L22.5 23h-3.7l-3.1-4.6H12V23zm3-7.4h4.9a1.9 1.9 0 0 0 0-3.8H12z"
        className="fill-primary-foreground"
      />
    </svg>
  )
}

/**
 * Renders the organization's uploaded/linked logo when set, falling back cleanly
 * to the default Rig mark if unset or if an external link fails to load.
 */
export function OrgMark({
  logoUrl,
  name,
  className,
}: {
  logoUrl?: string | null
  name?: string
  className?: string
}) {
  const [failed, setFailed] = useState(false)

  useEffect(() => {
    setFailed(false)
  }, [logoUrl])

  if (!logoUrl || failed) {
    return <Logo className={className} />
  }

  return (
    <img
      src={logoUrl}
      alt={name ? `${name} logo` : 'Organization logo'}
      referrerPolicy="no-referrer"
      onError={() => setFailed(true)}
      className={cn(
        'size-6 shrink-0 rounded-[7px] bg-surface object-cover ring-1 ring-border/70',
        className,
      )}
    />
  )
}
