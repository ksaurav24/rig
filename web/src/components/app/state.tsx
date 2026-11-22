import { CircleAlert, RotateCw, type LucideIcon } from 'lucide-react'
import type { ReactNode } from 'react'
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

const DOT: Record<Tone, string> = {
  ok: 'bg-ok',
  busy: 'bg-busy animate-pulse',
  warn: 'bg-warn',
  fail: 'bg-fail',
  idle: 'bg-idle',
}

/** A state dot with its label. Colour never carries the meaning alone. */
export function Status({ tone, children, className }: { tone: Tone; children: ReactNode; className?: string }) {
  return (
    <span className={cn('inline-flex items-center gap-1.5 text-[0.8125rem] whitespace-nowrap', className)}>
      <span aria-hidden="true" className={cn('size-2 shrink-0 rounded-full', DOT[tone])} />
      {children}
    </span>
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
    <Empty className="rounded-lg border border-dashed py-14">
      <EmptyHeader>
        <EmptyMedia variant="icon">
          <Icon />
        </EmptyMedia>
        <EmptyTitle>{title}</EmptyTitle>
        {children ? <EmptyDescription>{children}</EmptyDescription> : null}
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
