import type { LucideIcon } from 'lucide-react'
import type { ReactNode } from 'react'
import { Badge } from '../components/badge'
import { cn } from '../lib/utils'

export type Tone = 'ok' | 'busy' | 'warn' | 'fail' | 'idle'

const BADGE_COLOR: Record<Tone, 'emerald' | 'blue' | 'amber' | 'red' | 'gray'> = {
  ok: 'emerald',
  busy: 'blue',
  warn: 'amber',
  fail: 'red',
  idle: 'gray',
}

export function statusTone(status: string): Tone {
  switch (status.toLowerCase()) {
    case 'running':
    case 'online':
    case 'succeeded':
    case 'completed':
    case 'connected':
    case 'streaming':
      return 'ok'
    case 'deploying':
    case 'building':
    case 'queued':
    case 'connecting':
      return 'busy'
    case 'degraded':
    case 'warning':
      return 'warn'
    case 'failed':
    case 'crashed':
    case 'error':
      return 'fail'
    default:
      return 'idle'
  }
}

/** A state dot with its label, built on Fluid's Badge. Color never carries meaning alone. */
export function Status({
  tone,
  children,
  className,
  variant = 'ghost',
  size = 'default',
}: {
  tone: Tone
  children: ReactNode
  className?: string
  variant?: 'ghost' | 'dot' | 'solid'
  size?: 'default' | 'compact'
}) {
  return (
    <Badge
      variant={variant}
      color={BADGE_COLOR[tone]}
      size={size}
      className={cn(
        'text-[13px] font-normal',
        tone === 'busy' && '[&>span:first-child]:animate-pulse',
        className,
      )}
    >
      {children}
    </Badge>
  )
}

export function EmptyState({
  icon: Icon,
  title,
  children,
  action,
  className,
}: {
  icon: LucideIcon
  title: string
  children?: ReactNode
  action?: ReactNode
  className?: string
}) {
  return (
    <div
      className={cn(
        'flex flex-col items-center justify-center rounded-xl border border-dashed border-border/90 bg-card/50 px-6 py-12 text-center',
        className,
      )}
    >
      <div className="mb-3.5 flex size-11 items-center justify-center rounded-xl bg-hover text-muted-foreground shadow-[var(--shadow-1)]">
        <Icon className="size-5" strokeWidth={1.6} />
      </div>
      <div className="text-[15px] font-medium tracking-tight text-foreground">{title}</div>
      {children ? (
        <p className="mt-1.5 max-w-md text-[13.5px] leading-relaxed text-muted-foreground">
          {children}
        </p>
      ) : null}
      {action ? (
        <div className="mt-5 flex flex-wrap items-center justify-center gap-2">{action}</div>
      ) : null}
    </div>
  )
}
