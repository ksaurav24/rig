import type { ReactNode } from 'react'
import { cn } from '@/lib/utils'

/** Standard page frame: title row with actions, then content. */
export function Page({
  title,
  description,
  actions,
  children,
  className,
}: {
  title: string
  description?: ReactNode
  actions?: ReactNode
  children: ReactNode
  className?: string
}) {
  return (
    <div className={cn('mx-auto flex w-full max-w-6xl flex-col gap-6 px-4 py-6 md:px-8 md:py-8', className)}>
      <header className="flex flex-wrap items-center justify-between gap-x-6 gap-y-3">
        <div className="min-w-0">
          <h1 className="text-xl font-semibold tracking-tight text-foreground">{title}</h1>
          {description ? <p className="mt-1 max-w-[65ch] text-[13px] text-muted-foreground">{description}</p> : null}
        </div>
        {actions ? <div className="flex shrink-0 items-center gap-2">{actions}</div> : null}
      </header>
      {children}
    </div>
  )
}

/** A titled, surfaced block using Fluid Functionalism's shadow-hairline elevation. */
export function Panel({
  title,
  description,
  actions,
  children,
  className,
  flush = false,
  tone = 'default',
}: {
  title?: string
  description?: ReactNode
  actions?: ReactNode
  children: ReactNode
  className?: string
  /** Let content (tables, lists) run to the panel edge. */
  flush?: boolean
  /** Visual emphasis for destructive sections. */
  tone?: 'default' | 'danger'
}) {
  return (
    <section
      className={cn(
        'overflow-hidden rounded-xl bg-card',
        tone === 'danger'
          ? 'border border-fail/30 bg-fail/[0.03]'
          : 'shadow-[var(--shadow-2)]',
        className,
      )}
    >
      {title ? (
        <header
          className={cn(
            'flex flex-wrap items-center justify-between gap-4 border-b px-4 py-3',
            tone === 'danger' ? 'border-fail/20' : 'border-border/80',
          )}
        >
          <div className="min-w-0">
            <h2 className={cn('text-[14px] font-medium tracking-tight', tone === 'danger' && 'text-fail')}>
              {title}
            </h2>
            {description ? <p className="mt-0.5 text-[12.5px] text-muted-foreground">{description}</p> : null}
          </div>
          {actions}
        </header>
      ) : null}
      <div className={flush ? undefined : 'p-4'}>{children}</div>
    </section>
  )
}
