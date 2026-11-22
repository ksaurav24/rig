import { CircleAlert, RotateCw, type LucideIcon } from 'lucide-react'
import type { ReactNode } from 'react'
import { Badge } from '@/components/fluid/badge'
import { Button } from '@/components/ui/button'
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/ui/empty'
import { cn } from '@/lib/utils'

export type Tone = 'ok' | 'busy' | 'warn' | 'fail' | 'idle'

const BADGE_COLOR: Record<Tone, 'emerald' | 'blue' | 'amber' | 'red' | 'gray'> = {
  ok: 'emerald',
  busy: 'blue',
  warn: 'amber',
  fail: 'red',
  idle: 'gray',
}

/** A state dot with its label, built on Fluid's Badge. Colour never carries meaning alone. */
export function Status({
  tone,
  children,
  className,
  variant = 'ghost',
}: {
  tone: Tone
  children: ReactNode
  className?: string
  variant?: 'ghost' | 'dot' | 'solid'
}) {
  return (
    <Badge
      variant={variant}
      color={BADGE_COLOR[tone]}
      size="default"
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
}: {
  icon: LucideIcon
  title: string
  children?: ReactNode
  action?: ReactNode
}) {
  return (
    <Empty className="rounded-xl border border-dashed border-border/90 bg-card/40 py-12">
      <EmptyHeader>
        <EmptyMedia variant="icon">
          <Icon />
        </EmptyMedia>
        <EmptyTitle className="text-[15px] font-medium">{title}</EmptyTitle>
        {children ? <EmptyDescription className="max-w-md text-[13px]">{children}</EmptyDescription> : null}
      </EmptyHeader>
      {action ? <EmptyContent>{action}</EmptyContent> : null}
    </Empty>
  )
}

export function ErrorState({ error, onRetry }: { error: unknown; onRetry?: () => void }) {
  const message = error instanceof Error ? error.message : 'Something went wrong.'
  return (
    <EmptyState
      icon={CircleAlert}
      title="This did not load"
      action={
        onRetry ? (
          <Button variant="outline" size="sm" onClick={onRetry}>
            <RotateCw data-icon="inline-start" />
            Try again
          </Button>
        ) : undefined
      }
    >
      {message}
    </EmptyState>
  )
}
