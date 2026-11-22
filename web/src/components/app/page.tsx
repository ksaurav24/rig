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
      <header className="flex flex-wrap items-start justify-between gap-x-6 gap-y-3">
        <div className="min-w-0">
          <h1 className="text-xl font-semibold">{title}</h1>
          {description ? <p className="mt-1 max-w-[65ch] text-muted-foreground">{description}</p> : null}
        </div>
        {actions ? <div className="flex shrink-0 items-center gap-2">{actions}</div> : null}
      </header>
      {children}
    </div>
  )
}

/** A titled, bordered block. Hierarchy comes from borders, not shadows. */
export function Panel({
  title,
  description,
  actions,
  children,
  className,
  flush = false,
}: {
  title?: string
  description?: ReactNode
  actions?: ReactNode
  children: ReactNode
  className?: string
  /** Let content (tables, lists) run to the panel edge. */
  flush?: boolean
}) {
  return (
    <section className={cn('overflow-hidden rounded-lg border bg-card', className)}>
      {title ? (
        <header className="flex items-center justify-between gap-4 border-b px-4 py-3">
          <div className="min-w-0">
            <h2 className="font-medium">{title}</h2>
            {description ? <p className="text-[0.8125rem] text-muted-foreground">{description}</p> : null}
          </div>
          {actions}
        </header>
      ) : null}
      <div className={flush ? undefined : 'p-4'}>{children}</div>
    </section>
  )
}
