import type { ReactNode } from 'react'
import { Logo } from '@/components/app/logo'

/** Frame for the two signed-out screens. */
export function AuthLayout({ title, description, children }: { title: string; description: ReactNode; children: ReactNode }) {
  return (
    <main className="grid min-h-svh place-items-center bg-surface px-4 py-10">
      <div className="w-full max-w-sm">
        <div className="mb-6 flex items-center gap-2.5">
          <Logo className="size-7" />
          <span className="text-base font-semibold">Rig</span>
        </div>
        <div className="rounded-lg border bg-card p-6">
          <h1 className="text-lg font-semibold">{title}</h1>
          <p className="mt-1 mb-5 text-muted-foreground">{description}</p>
          {children}
        </div>
      </div>
    </main>
  )
}
