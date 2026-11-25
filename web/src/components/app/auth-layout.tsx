import { ArrowUpRight, CheckCircle2, Database, GitBranch, Lock, ShieldCheck, Terminal } from 'lucide-react'
import { motion, useReducedMotion } from 'motion/react'
import type { ReactNode } from 'react'
import type { Schemas } from '@/api/client'
import { OrgMark } from '@rig/ui/app/logo'
import { ThemeToggle } from '@/components/app/theme-toggle'
import { Badge } from '@rig/ui/badge'
import { isEnvBadgeColor } from '@/lib/preferences'
import { cn } from '@rig/ui/lib/utils'

export interface AuthLayoutProps {
  mode: 'login' | 'setup'
  title: string
  description: ReactNode
  eyebrow?: string
  branding?: Schemas['PublicBranding'] | null
  version?: string
  children: ReactNode
}

/**
 * Split-screen layout for signed-out screens (Login & Initial Setup):
 * - Left: brand identity, editorial context, and an architectural system preview.
 * - Right: focused auth form with clean hierarchy and theme toggle.
 */
export function AuthLayout({
  mode,
  title,
  description,
  eyebrow,
  branding,
  version,
  children,
}: AuthLayoutProps) {
  const reduceMotion = useReducedMotion()
  const brandName = branding?.org_name?.trim() || 'Rig'
  const logoUrl = branding?.logo_url ?? null
  const envLabel = branding?.env_label ?? null
  const envColor = isEnvBadgeColor(branding?.env_color) ? branding.env_color : 'emerald'

  return (
    <div className="grid min-h-dvh bg-background lg:grid-cols-[minmax(0,1.08fr)_minmax(0,1fr)]">
      {/* Left showcase panel */}
      <aside className="relative hidden flex-col justify-between overflow-hidden border-r border-border/80 bg-surface p-10 xl:p-14 lg:flex">
        {/* Subtle architectural grid & accent ambient glow */}
        <div
          aria-hidden="true"
          className="pointer-events-none absolute inset-0 opacity-[0.42] dark:opacity-[0.28]"
          style={{
            backgroundImage:
              'linear-gradient(to right, var(--border) 1px, transparent 1px), linear-gradient(to bottom, var(--border) 1px, transparent 1px)',
            backgroundSize: '44px 44px',
            maskImage: 'radial-gradient(ellipse 85% 75% at 35% 35%, #000 30%, transparent 100%)',
            WebkitMaskImage: 'radial-gradient(ellipse 85% 75% at 35% 35%, #000 30%, transparent 100%)',
          }}
        />
        <div
          aria-hidden="true"
          className="pointer-events-none absolute -top-28 -left-24 size-[460px] rounded-full blur-3xl"
          style={{
            background: 'radial-gradient(circle, color-mix(in oklab, var(--primary) 20%, transparent), transparent 68%)',
          }}
        />
        <div
          aria-hidden="true"
          className="pointer-events-none absolute -right-24 -bottom-32 size-[420px] rounded-full blur-3xl"
          style={{
            background: 'radial-gradient(circle, color-mix(in oklab, var(--primary) 12%, transparent), transparent 70%)',
          }}
        />

        {/* Top brand bar */}
        <div className="relative z-10 flex items-center justify-between gap-4">
          <div className="flex min-w-0 items-center gap-3">
            <OrgMark logoUrl={logoUrl} name={brandName} className="size-8 rounded-lg shadow-xs" />
            <div className="flex min-w-0 items-center gap-2">
              <span className="truncate text-[15px] font-semibold tracking-tight text-foreground">
                {brandName}
              </span>
              {envLabel ? (
                <Badge variant="solid" color={envColor} size="compact">
                  {envLabel}
                </Badge>
              ) : null}
            </div>
          </div>
          {version ? (
            <span className="rounded-full border border-border/80 bg-card/80 px-2.5 py-0.5 font-mono text-[11px] text-muted-foreground backdrop-blur-xs">
              v{version}
            </span>
          ) : null}
        </div>

        {/* Center editorial content & visual preview */}
        <motion.div
          initial={reduceMotion ? false : { opacity: 0, y: 12 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.35, ease: [0.22, 1, 0.36, 1] }}
          className="relative z-10 my-auto max-w-lg py-10"
        >
          <div className="mb-4 inline-flex items-center gap-2 rounded-full border border-border/80 bg-card/85 px-3 py-1 text-[12px] font-medium text-muted-foreground shadow-[var(--shadow-1)] backdrop-blur-xs">
            <span className="size-1.5 rounded-full bg-primary" />
            {mode === 'setup' ? 'First-time server initialization' : 'Self-hosted deployment platform'}
          </div>

          <h2 className="text-[28px] leading-[1.16] font-semibold tracking-[-0.025em] text-foreground xl:text-[32px]">
            {mode === 'setup'
              ? 'Your infrastructure, one calm control plane.'
              : branding?.org_name
                ? `Welcome back to ${branding.org_name}.`
                : 'Ship apps and databases without the ceremony.'}
          </h2>

          <p className="mt-3 text-[14px] leading-relaxed text-muted-foreground">
            {mode === 'setup'
              ? 'Claim the initial owner account with the one-time token printed in your server console. Everything stays on your machine.'
              : 'Deploy from Git or container images, manage encrypted databases, and inspect live logs and terminals from a single binary.'}
          </p>

          {mode === 'setup' ? <SetupVisualCard /> : <LoginVisualCard brandName={brandName} />}
        </motion.div>

        {/* Bottom capability strip */}
        <div className="relative z-10 flex flex-wrap items-center gap-x-5 gap-y-2 border-t border-border/70 pt-5 text-[12px] text-muted-foreground">
          <span className="inline-flex items-center gap-1.5">
            <ShieldCheck className="size-3.5 text-primary" />
            2FA &amp; audit logging
          </span>
          <span className="inline-flex items-center gap-1.5">
            <Lock className="size-3.5 text-primary" />
            Automatic HTTPS
          </span>
          <span className="inline-flex items-center gap-1.5">
            <Terminal className="size-3.5 text-primary" />
            CLI &amp; MCP ready
          </span>
        </div>
      </aside>

      {/* Right auth form panel */}
      <div className="flex min-h-dvh flex-col bg-background">
        <header className="flex h-14 shrink-0 items-center justify-between px-5 sm:px-8">
          <div className="flex min-w-0 items-center gap-2.5 lg:invisible">
            <OrgMark logoUrl={logoUrl} name={brandName} className="size-7 rounded-lg" />
            <span className="truncate text-[14.5px] font-semibold tracking-tight text-foreground">
              {brandName}
            </span>
            {envLabel ? (
              <Badge variant="solid" color={envColor} size="compact">
                {envLabel}
              </Badge>
            ) : null}
          </div>
          <ThemeToggle />
        </header>

        <main className="flex flex-1 items-center justify-center px-5 py-8 sm:px-8">
          <motion.div
            initial={reduceMotion ? false : { opacity: 0, y: 8 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.28, ease: [0.22, 1, 0.36, 1] }}
            className="w-full max-w-[400px]"
          >
            {eyebrow ? (
              <p className="mb-2 text-[12px] font-medium tracking-wide text-primary uppercase">
                {eyebrow}
              </p>
            ) : null}
            <h1 className="text-[22px] font-semibold tracking-[-0.02em] text-foreground">{title}</h1>
            <p className="mt-1.5 mb-6 text-[13.5px] leading-relaxed text-muted-foreground">{description}</p>
            {children}
          </motion.div>
        </main>

        <footer className="flex h-12 shrink-0 items-center justify-between px-5 text-[11.5px] text-muted-foreground sm:px-8">
          <span>{brandName}</span>
          <span className="font-mono">{version ? `Rig v${version}` : 'Self-hosted'}</span>
        </footer>
      </div>
    </div>
  )
}

function LoginVisualCard({ brandName }: { brandName: string }) {
  const services = [
    { name: 'api-gateway', meta: 'main · zero-downtime', status: 'Running', icon: GitBranch },
    { name: 'web-dashboard', meta: 'auto-https · caddy', status: 'Running', icon: ArrowUpRight },
    { name: 'primary-postgres', meta: 'pg 17 · encrypted backups', status: 'Healthy', icon: Database },
  ]

  return (
    <div className="mt-8 overflow-hidden rounded-2xl border border-border/80 bg-card/90 shadow-[var(--shadow-4)] backdrop-blur-sm">
      <div className="flex items-center justify-between border-b border-border/80 bg-surface/60 px-4 py-2.5">
        <div className="flex items-center gap-2">
          <span className="size-2 rounded-full bg-ok" />
          <span className="font-mono text-[11.5px] font-medium text-foreground">
            {brandName.toLowerCase().replace(/[^a-z0-9]+/g, '-') || 'rig'} / production
          </span>
        </div>
        <span className="font-mono text-[11px] text-muted-foreground">3/3 healthy</span>
      </div>

      <div className="divide-y divide-border/70">
        {services.map((svc) => {
          const Icon = svc.icon
          return (
            <div key={svc.name} className="flex items-center justify-between gap-3 px-4 py-3">
              <div className="flex min-w-0 items-center gap-3">
                <span className="grid size-7 shrink-0 place-items-center rounded-lg bg-surface text-muted-foreground shadow-[0_0_0_1px_var(--border)]">
                  <Icon className="size-3.5" />
                </span>
                <div className="min-w-0">
                  <div className="truncate text-[13px] font-medium text-foreground">{svc.name}</div>
                  <div className="truncate font-mono text-[11px] text-muted-foreground">{svc.meta}</div>
                </div>
              </div>
              <span className="inline-flex shrink-0 items-center gap-1.5 rounded-full bg-ok/10 px-2 py-0.5 text-[11px] font-medium text-ok">
                <span className="size-1.5 rounded-full bg-ok" />
                {svc.status}
              </span>
            </div>
          )
        })}
      </div>

      <div className="flex items-center justify-between border-t border-border/80 bg-surface/50 px-4 py-2 font-mono text-[11px] text-muted-foreground">
        <span>$ rig deploy --watch</span>
        <span className="text-primary">switched traffic in 1.2s</span>
      </div>
    </div>
  )
}

function SetupVisualCard() {
  const steps = [
    {
      num: '01',
      title: 'Claim owner account',
      body: 'Paste the one-time setup token from your server terminal to initialize your organization.',
      active: true,
    },
    {
      num: '02',
      title: 'Connect GitHub or container images',
      body: 'Deploy directly from repositories with push-to-deploy or pull any OCI image.',
      active: false,
    },
    {
      num: '03',
      title: 'Automatic HTTPS & live ops',
      body: 'Caddy provisions certificates automatically while you monitor metrics, logs, and backups.',
      active: false,
    },
  ]

  return (
    <div className="mt-8 space-y-2.5">
      {steps.map((step) => (
        <div
          key={step.num}
          className={cn(
            'flex items-start gap-3.5 rounded-xl border px-4 py-3 transition-colors',
            step.active
              ? 'border-primary/40 bg-card shadow-[var(--shadow-2)]'
              : 'border-border/75 bg-card/65',
          )}
        >
          <span
            className={cn(
              'mt-0.5 grid size-6 shrink-0 place-items-center rounded-md font-mono text-[11px] font-semibold',
              step.active
                ? 'bg-primary text-primary-foreground'
                : 'bg-surface text-muted-foreground ring-1 ring-border',
            )}
          >
            {step.num}
          </span>
          <div className="min-w-0">
            <div className="flex items-center gap-1.5 text-[13px] font-medium text-foreground">
              {step.title}
              {step.active ? <CheckCircle2 className="size-3.5 text-primary" /> : null}
            </div>
            <p className="mt-0.5 text-[12px] leading-relaxed text-muted-foreground">{step.body}</p>
          </div>
        </div>
      ))}
    </div>
  )
}
