import { queryOptions, useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute, Link, useNavigate } from '@tanstack/react-router'
import {
  ArrowRight,
  Camera,
  Check,
  Clock,
  Home,
  KeyRound,
  Lock,
  LogOut,
  Monitor,
  Moon,
  Palette,
  PanelLeft,
  RotateCcw,
  ShieldCheck,
  Sliders,
  Square,
  Sun,
  SunMoon,
  TerminalSquare,
  type LucideIcon,
} from 'lucide-react'
import { useEffect, useState, type FormEvent, type ReactNode } from 'react'
import { api, unwrap, type Schemas } from '@/api/client'
import { membersQuery, meQuery, tokensQuery } from '@/api/queries'
import { CopyButton } from '@/components/app/copy-button'
import { ImagePicker } from '@/components/app/image-picker'
import { Page, Panel } from '@rig/ui/app/page'
import { UserAvatar } from '@/components/app/user-avatar'
import { Badge } from '@rig/ui/badge'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@rig/ui/dialog'
import { Field, FieldDescription, FieldError, FieldGroup, FieldLabel } from '@rig/ui/field'
import { Input } from '@rig/ui/input'
import { Switch } from '@rig/ui/switch'
import { Tabs, TabsItem } from '@rig/ui/tabs'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { fullDate, timeAgo } from '@/lib/format'
import {
  ACCENT_PRESETS,
  type AccentChoice,
  type DefaultHomeRoute,
  type HourCycleMode,
  type LogFontSize,
  type SidebarStyleVariant,
  type TimeStyle,
  type TimeZoneMode,
  usePreferences,
} from '@/lib/preferences'
import type { ShapeVariant } from '@rig/ui/lib/shape-context'
import type { SizeVariant } from '@rig/ui/lib/size-context'
import { useTheme, type Theme } from '@/lib/theme'
import { cn } from '@rig/ui/lib/utils'

export const Route = createFileRoute('/_app/settings/account')({
  staticData: { crumb: 'Account' },
  loader: ({ context }) => {
    void context.queryClient.prefetchQuery(tokensQuery)
  },
  component: AccountPage,
})

const THEME_OPTIONS: { value: Theme; label: string; icon: LucideIcon }[] = [
  { value: 'light', label: 'Light', icon: Sun },
  { value: 'dark', label: 'Dark', icon: Moon },
  { value: 'system', label: 'System', icon: Monitor },
]

const DENSITY_OPTIONS: { value: SizeVariant; label: string }[] = [
  { value: 'default', label: 'Default' },
  { value: 'compact', label: 'Compact' },
]

const SHAPE_OPTIONS: { value: ShapeVariant; label: string }[] = [
  { value: 'rounded', label: 'Rounded' },
  { value: 'sharp', label: 'Sharp' },
  { value: 'pill', label: 'Pill' },
]

const SIDEBAR_VARIANT_OPTIONS: { value: SidebarStyleVariant; label: string }[] = [
  { value: 'sidebar', label: 'Classic' },
  { value: 'floating', label: 'Floating' },
  { value: 'inset', label: 'Inset' },
]

const HOME_OPTIONS: { value: DefaultHomeRoute; label: string }[] = [
  { value: '/', label: 'Overview' },
  { value: '/projects', label: 'Projects' },
  { value: '/databases', label: 'Databases' },
]

const TIME_STYLE_OPTIONS: { value: TimeStyle; label: string }[] = [
  { value: 'relative', label: 'Relative' },
  { value: 'exact', label: 'Exact' },
]

const TZ_OPTIONS: { value: TimeZoneMode; label: string }[] = [
  { value: 'local', label: 'Local' },
  { value: 'utc', label: 'UTC' },
]

const HOUR_OPTIONS: { value: HourCycleMode; label: string }[] = [
  { value: 'auto', label: 'Auto' },
  { value: '12h', label: '12h' },
  { value: '24h', label: '24h' },
]

const LOG_FONT_OPTIONS: { value: LogFontSize; label: string }[] = [
  { value: 12, label: '12px' },
  { value: 13, label: '13px' },
  { value: 14, label: '14px' },
]

function AccountPage() {
  const me = useQuery(meQuery).data
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const [editingProfile, setEditingProfile] = useState(false)

  const signOut = useMutation({
    mutationFn: () => unwrap(api.POST('/api/v1/auth/logout')),
    onSettled: async () => {
      queryClient.clear()
      await navigate({ to: '/login' })
    },
  })

  return (
    <Page title="Account" className="max-w-3xl">
      <Panel>
        <div className="flex flex-wrap items-center gap-4">
          <button
            type="button"
            onClick={() => setEditingProfile(true)}
            aria-label="Change avatar and profile"
            className="group relative rounded-xl focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <UserAvatar
              name={me?.user.name ?? '?'}
              avatarUrl={me?.user.avatar_url}
              className="size-14 rounded-xl"
              fallbackClassName="text-sm font-semibold"
            />
            <span className="absolute -right-1 -bottom-1 grid size-6 place-items-center rounded-full border border-border bg-card text-muted-foreground shadow-xs transition-colors group-hover:bg-primary group-hover:text-primary-foreground">
              <Camera className="size-3" />
            </span>
          </button>

          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center gap-2">
              <h2 className="truncate text-[15px] font-semibold text-foreground">{me?.user.name}</h2>
              {me?.role ? (
                <Badge variant="solid" color="emerald" size="compact" className="capitalize">
                  {me.role}
                </Badge>
              ) : null}
            </div>
            <p className="mt-0.5 truncate text-[13px] text-muted-foreground">
              {me?.user.email}
              {me?.org.name ? <span> · {me.org.name}</span> : null}
            </p>
          </div>

          <div className="flex flex-wrap items-center gap-2">
            <Button variant="outline" size="sm" onClick={() => setEditingProfile(true)}>
              Edit profile
            </Button>
            <Button
              variant="outline"
              size="sm"
              disabled={signOut.isPending}
              onClick={() => signOut.mutate()}
            >
              <LogOut data-icon="inline-start" />
              Sign out
            </Button>
          </div>
        </div>
      </Panel>

      {me ? (
        <EditProfileDialog open={editingProfile} onOpenChange={setEditingProfile} user={me.user} />
      ) : null}

      <AppearancePanel />
      <WorkspacePrefsPanel />
      <SecurityPanel />
    </Page>
  )
}

function EditProfileDialog({
  open,
  onOpenChange,
  user,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  user: Schemas['User']
}) {
  const queryClient = useQueryClient()
  const [name, setName] = useState(user.name)

  useEffect(() => {
    if (open) setName(user.name)
  }, [open, user.name])

  function applyUser(updated: Schemas['User']) {
    queryClient.setQueryData<Schemas['Me']>(meQuery.queryKey, (old) => (old ? { ...old, user: updated } : old))
    void queryClient.invalidateQueries({ queryKey: membersQuery.queryKey })
  }

  const updateProfile = useMutation({
    mutationFn: (body: Schemas['UpdateProfileRequest']) =>
      unwrap(api.PATCH('/api/v1/account/profile', { body })),
    meta: { success: 'Profile updated' },
    onSuccess: applyUser,
  })

  const uploadAvatar = useMutation({
    mutationFn: (body: Schemas['UploadImageRequest']) =>
      unwrap(api.POST('/api/v1/account/avatar', { body })),
    meta: { success: 'Avatar updated' },
    onSuccess: applyUser,
  })

  const removeAvatar = useMutation({
    mutationFn: () => unwrap(api.DELETE('/api/v1/account/avatar')),
    meta: { success: 'Avatar reset to initials' },
    onSuccess: applyUser,
  })

  const busy = updateProfile.isPending || uploadAvatar.isPending || removeAvatar.isPending

  function submitName(e: FormEvent<HTMLFormElement>) {
    e.preventDefault()
    const trimmed = name.trim()
    if (!trimmed || trimmed === user.name) return
    updateProfile.mutate({ name: trimmed })
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent size="lg">
        <DialogHeader>
          <DialogTitle>Edit profile</DialogTitle>
          <DialogDescription>
            Update your display name or choose an avatar from presets, an uploaded image, or an image link.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-5">
          <form onSubmit={submitName} className="flex flex-col gap-3 sm:flex-row sm:items-end">
            <UserAvatar
              name={name.trim() || user.name}
              avatarUrl={user.avatar_url}
              className="size-12 shrink-0 rounded-xl"
              fallbackClassName="text-sm font-semibold"
            />
            <Field className="flex-1">
              <FieldLabel htmlFor="profile-name">Display name</FieldLabel>
              <Input
                id="profile-name"
                value={name}
                onChange={(e) => setName(e.target.value)}
                required
                maxLength={80}
              />
            </Field>
            <Button
              type="submit"
              size="sm"
              disabled={busy || !name.trim() || name.trim() === user.name}
            >
              {updateProfile.isPending ? <Spinner data-icon="inline-start" /> : null}
              Save name
            </Button>
          </form>

          <div className="border-t border-border/80 pt-4">
            <p className="mb-2.5 text-[13px] font-medium text-foreground">Avatar</p>
            <ImagePicker
              currentUrl={user.avatar_url}
              showPresets
              square
              pending={busy}
              onUpload={(payload) => uploadAvatar.mutate(payload)}
              onSetUrl={(avatar_url) => updateProfile.mutate({ avatar_url })}
              onSelectPreset={(avatar_url) => updateProfile.mutate({ avatar_url })}
              onRemove={() => removeAvatar.mutate()}
              removeLabel="Reset to initials"
            />
          </div>
        </div>

        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
            Done
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function SettingRow({
  icon: Icon,
  title,
  badge,
  description,
  actions,
}: {
  icon: LucideIcon
  title: string
  badge?: ReactNode
  description: ReactNode
  actions: ReactNode
}) {
  return (
    <div className="flex flex-col gap-3 px-4 py-3.5 sm:flex-row sm:items-center sm:justify-between">
      <div className="flex min-w-0 items-start gap-3">
        <span className="mt-0.5 grid size-8 shrink-0 place-items-center rounded-lg bg-surface text-muted-foreground shadow-[0_0_0_1px_var(--border)]">
          <Icon className="size-4" aria-hidden />
        </span>
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-[13.5px] font-medium text-foreground">{title}</span>
            {badge}
          </div>
          <p className="mt-0.5 text-[12.5px] text-muted-foreground">{description}</p>
        </div>
      </div>
      <div className="flex shrink-0 flex-wrap items-center gap-2 pl-11 sm:pl-0">{actions}</div>
    </div>
  )
}

function AppearancePanel() {
  const { theme, setTheme, resolved } = useTheme()
  const { prefs, orgAccent, updatePrefs } = usePreferences()

  const themeIndex = Math.max(0, THEME_OPTIONS.findIndex((t) => t.value === theme))
  const densityIndex = Math.max(0, DENSITY_OPTIONS.findIndex((d) => d.value === prefs.density))
  const shapeIndex = Math.max(0, SHAPE_OPTIONS.findIndex((s) => s.value === prefs.shape))
  const sidebarIndex = Math.max(
    0,
    SIDEBAR_VARIANT_OPTIONS.findIndex((v) => v.value === prefs.sidebarVariant),
  )

  const orgPreset = ACCENT_PRESETS.find((a) => a.value === orgAccent) ?? ACCENT_PRESETS[0]
  const orgSwatch = resolved === 'dark' ? orgPreset.swatchDark : orgPreset.swatchLight

  return (
    <Panel title="Appearance" description="Stored on this browser." flush>
      <div className="divide-y divide-border/80">
        <SettingRow
          icon={SunMoon}
          title="Theme"
          description="Light, dark, or match your operating system."
          actions={
            <Tabs
              size="compact"
              selectedIndex={themeIndex}
              onSelect={(i) => setTheme(THEME_OPTIONS[i].value)}
              aria-label="Theme"
              className="border border-border/80 bg-surface/60"
            >
              {THEME_OPTIONS.map((opt, i) => (
                <TabsItem key={opt.value} index={i} icon={opt.icon} label={opt.label} />
              ))}
            </Tabs>
          }
        />

        <div className="px-4 py-3.5">
          <div className="flex items-start gap-3">
            <span className="mt-0.5 grid size-8 shrink-0 place-items-center rounded-lg bg-surface text-muted-foreground shadow-[0_0_0_1px_var(--border)]">
              <Palette className="size-4" aria-hidden />
            </span>
            <div className="min-w-0 flex-1">
              <span className="text-[13.5px] font-medium text-foreground">Accent color</span>
              <p className="mt-0.5 text-[12.5px] text-muted-foreground">
                Follow the organization accent ({orgPreset.label}) or pick a personal override.
              </p>
              <div className="mt-3 flex flex-wrap items-center gap-1.5" role="radiogroup" aria-label="Accent color">
                <button
                  type="button"
                  role="radio"
                  aria-checked={prefs.accent === 'org'}
                  onClick={() => updatePrefs({ accent: 'org' })}
                  className={cn(
                    'inline-flex h-8 items-center gap-2 rounded-full border px-3 text-[12.5px] transition-colors',
                    prefs.accent === 'org'
                      ? 'border-primary bg-primary/[0.08] font-medium text-foreground'
                      : 'border-border/80 bg-card text-muted-foreground hover:text-foreground',
                  )}
                >
                  <span
                    className="size-3 rounded-full ring-1 ring-border/60"
                    style={{ backgroundColor: orgSwatch }}
                  />
                  Org default
                </button>
                {ACCENT_PRESETS.map((preset) => {
                  const active = prefs.accent === preset.value
                  const swatch = resolved === 'dark' ? preset.swatchDark : preset.swatchLight
                  return (
                    <button
                      key={preset.value}
                      type="button"
                      role="radio"
                      aria-checked={active}
                      title={preset.label}
                      onClick={() => updatePrefs({ accent: preset.value as AccentChoice })}
                      className={cn(
                        'inline-flex h-8 items-center gap-1.5 rounded-full border px-2.5 text-[12.5px] transition-colors',
                        active
                          ? 'border-primary bg-primary/[0.08] font-medium text-foreground'
                          : 'border-border/80 bg-card text-muted-foreground hover:text-foreground',
                      )}
                    >
                      <span
                        className="grid size-3.5 place-items-center rounded-full"
                        style={{ backgroundColor: swatch }}
                      >
                        {active ? <Check className="size-2.5 text-white mix-blend-difference" /> : null}
                      </span>
                      {preset.label}
                    </button>
                  )
                })}
              </div>
            </div>
          </div>
        </div>

        <SettingRow
          icon={Sliders}
          title="UI density"
          description="Compact reduces control heights and padding for denser tables and toolbars."
          actions={
            <Tabs
              size="compact"
              selectedIndex={densityIndex}
              onSelect={(i) => updatePrefs({ density: DENSITY_OPTIONS[i].value })}
              aria-label="UI density"
              className="border border-border/80 bg-surface/60"
            >
              {DENSITY_OPTIONS.map((opt, i) => (
                <TabsItem key={opt.value} index={i} label={opt.label} />
              ))}
            </Tabs>
          }
        />

        <SettingRow
          icon={Square}
          title="Corner style"
          description="Adjust border radius across controls, menus, and navigation."
          actions={
            <Tabs
              size="compact"
              selectedIndex={shapeIndex}
              onSelect={(i) => updatePrefs({ shape: SHAPE_OPTIONS[i].value })}
              aria-label="Corner style"
              className="border border-border/80 bg-surface/60"
            >
              {SHAPE_OPTIONS.map((opt, i) => (
                <TabsItem key={opt.value} index={i} label={opt.label} />
              ))}
            </Tabs>
          }
        />

        <SettingRow
          icon={PanelLeft}
          title="Sidebar layout"
          description="Choose classic, floating, or inset framing, and hover-peek when collapsed."
          actions={
            <div className="flex flex-wrap items-center gap-3">
              <Tabs
                size="compact"
                selectedIndex={sidebarIndex}
                onSelect={(i) => updatePrefs({ sidebarVariant: SIDEBAR_VARIANT_OPTIONS[i].value })}
                aria-label="Sidebar style"
                className="border border-border/80 bg-surface/60"
              >
                {SIDEBAR_VARIANT_OPTIONS.map((opt, i) => (
                  <TabsItem key={opt.value} index={i} label={opt.label} />
                ))}
              </Tabs>
              <Switch
                size="compact"
                label="Hover peek"
                checked={prefs.sidebarPeek === 'hover'}
                onToggle={() => updatePrefs({ sidebarPeek: prefs.sidebarPeek === 'hover' ? 'none' : 'hover' })}
              />
            </div>
          }
        />
      </div>
    </Panel>
  )
}

function WorkspacePrefsPanel() {
  const { prefs, updatePrefs, resetPrefs } = usePreferences()
  const tokens = useQuery(tokensQuery)

  const activeTokens =
    tokens.data?.filter((t) => !t.expires_at || new Date(t.expires_at).getTime() >= Date.now()).length ?? 0
  const expiredTokens = (tokens.data?.length ?? 0) - activeTokens

  const homeIndex = Math.max(0, HOME_OPTIONS.findIndex((o) => o.value === prefs.defaultHome))
  const timeStyleIndex = Math.max(0, TIME_STYLE_OPTIONS.findIndex((o) => o.value === prefs.timeStyle))
  const tzIndex = Math.max(0, TZ_OPTIONS.findIndex((o) => o.value === prefs.timeZone))
  const hourIndex = Math.max(0, HOUR_OPTIONS.findIndex((o) => o.value === prefs.hourCycle))
  const fontIndex = Math.max(0, LOG_FONT_OPTIONS.findIndex((o) => o.value === prefs.logFontSize))

  const sampleIso = new Date(Date.now() - 4 * 60_000).toISOString()

  return (
    <Panel
      title="Workspace & personal"
      actions={
        <Button type="button" variant="ghost" size="sm" onClick={resetPrefs}>
          <RotateCcw data-icon="inline-start" />
          Reset defaults
        </Button>
      }
      flush
    >
      <div className="divide-y divide-border/80">
        <SettingRow
          icon={Home}
          title="Default home page"
          description="Where signing in or opening the dashboard lands first."
          actions={
            <Tabs
              size="compact"
              selectedIndex={homeIndex}
              onSelect={(i) => updatePrefs({ defaultHome: HOME_OPTIONS[i].value })}
              aria-label="Default home page"
              className="border border-border/80 bg-surface/60"
            >
              {HOME_OPTIONS.map((opt, i) => (
                <TabsItem key={opt.value} index={i} label={opt.label} />
              ))}
            </Tabs>
          }
        />

        <div className="px-4 py-3.5">
          <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
            <div className="flex min-w-0 items-start gap-3">
              <span className="mt-0.5 grid size-8 shrink-0 place-items-center rounded-lg bg-surface text-muted-foreground shadow-[0_0_0_1px_var(--border)]">
                <Clock className="size-4" aria-hidden />
              </span>
              <div className="min-w-0">
                <span className="text-[13.5px] font-medium text-foreground">Timestamps</span>
                <p className="mt-0.5 text-[12.5px] text-muted-foreground">
                  Preview: <span className="font-mono text-foreground">{timeAgo(sampleIso)}</span>
                  <span className="mx-1.5">·</span>
                  <span className="font-mono">{fullDate(sampleIso)}</span>
                </p>
              </div>
            </div>
            <div className="flex flex-wrap items-center gap-2 pl-11 sm:justify-end sm:pl-0">
              <Tabs
                size="compact"
                selectedIndex={timeStyleIndex}
                onSelect={(i) => updatePrefs({ timeStyle: TIME_STYLE_OPTIONS[i].value })}
                aria-label="Timestamp style"
                className="border border-border/80 bg-surface/60"
              >
                {TIME_STYLE_OPTIONS.map((opt, i) => (
                  <TabsItem key={opt.value} index={i} label={opt.label} />
                ))}
              </Tabs>
              <Tabs
                size="compact"
                selectedIndex={tzIndex}
                onSelect={(i) => updatePrefs({ timeZone: TZ_OPTIONS[i].value })}
                aria-label="Timezone"
                className="border border-border/80 bg-surface/60"
              >
                {TZ_OPTIONS.map((opt, i) => (
                  <TabsItem key={opt.value} index={i} label={opt.label} />
                ))}
              </Tabs>
              <Tabs
                size="compact"
                selectedIndex={hourIndex}
                onSelect={(i) => updatePrefs({ hourCycle: HOUR_OPTIONS[i].value })}
                aria-label="Clock format"
                className="border border-border/80 bg-surface/60"
              >
                {HOUR_OPTIONS.map((opt, i) => (
                  <TabsItem key={opt.value} index={i} label={opt.label} />
                ))}
              </Tabs>
            </div>
          </div>
        </div>

        <SettingRow
          icon={TerminalSquare}
          title="Logs & terminal"
          description="Monospace font size and default line-wrapping in log panes and shell sessions."
          actions={
            <div className="flex flex-wrap items-center gap-3">
              <Tabs
                size="compact"
                selectedIndex={fontIndex}
                onSelect={(i) => updatePrefs({ logFontSize: LOG_FONT_OPTIONS[i].value })}
                aria-label="Log and terminal font size"
                className="border border-border/80 bg-surface/60"
              >
                {LOG_FONT_OPTIONS.map((opt, i) => (
                  <TabsItem key={opt.value} index={i} label={opt.label} />
                ))}
              </Tabs>
              <Switch
                size="compact"
                label="Wrap lines"
                checked={prefs.logWrap}
                onToggle={() => updatePrefs({ logWrap: !prefs.logWrap })}
              />
            </div>
          }
        />

        <SettingRow
          icon={KeyRound}
          title="Access tokens"
          description={
            tokens.isPending ? (
              'Personal tokens for the CLI, MCP clients, and scripts.'
            ) : tokens.data && tokens.data.length > 0 ? (
              `${activeTokens} active token${activeTokens === 1 ? '' : 's'}${
                expiredTokens > 0 ? ` · ${expiredTokens} expired` : ''
              }.`
            ) : (
              'No access tokens created yet.'
            )
          }
          actions={
            <Button variant="outline" size="sm" nativeButton={false} render={<Link to="/settings/tokens" />}>
              Manage tokens
              <ArrowRight data-icon="inline-end" />
            </Button>
          }
        />
      </div>
    </Panel>
  )
}

function SecurityPanel() {
  const [changingPassword, setChangingPassword] = useState(false)
  const queryClient = useQueryClient()
  const status = useQuery(totpQuery)
  const [step, setStep] = useState<Step>({ kind: 'idle' })
  const close = () => setStep({ kind: 'idle' })
  const refresh = () => queryClient.invalidateQueries({ queryKey: totpQuery.queryKey })

  const setup = useMutation({
    mutationFn: (password: string) => unwrap(api.POST('/api/v1/account/totp/setup', { body: { password } })),
    onSuccess: (enrollment) => setStep({ kind: 'scan', enrollment }),
  })
  const enable = useMutation({
    mutationFn: (code: string) => unwrap(api.POST('/api/v1/account/totp/enable', { body: { code } })),
    meta: { success: 'Two-factor sign-in is on. Other sessions were signed out.' },
    onSuccess: ({ codes }) => {
      setStep({ kind: 'codes', codes })
      void refresh()
    },
  })
  const disable = useMutation({
    mutationFn: (body: Schemas['TotpDisable']) => unwrap(api.POST('/api/v1/account/totp/disable', { body })),
    meta: { success: 'Two-factor sign-in is off.' },
    onSuccess: () => {
      close()
      void refresh()
    },
  })
  const regenerate = useMutation({
    mutationFn: (body: Schemas['TotpDisable']) => unwrap(api.POST('/api/v1/account/totp/recovery-codes', { body })),
    meta: { success: 'New recovery codes made. The old ones no longer work.' },
    onSuccess: ({ codes }) => {
      setStep({ kind: 'codes', codes })
      void refresh()
    },
  })

  const data = status.data
  const lowCodes = data?.enabled && data.recovery_codes_left <= 3

  return (
    <Panel title="Security" flush>
      <div className="divide-y divide-border/80">
        <SettingRow
          icon={Lock}
          title="Password"
          description="Updating your password signs out other browsers. Access tokens stay active."
          actions={
            <Button variant="outline" size="sm" onClick={() => setChangingPassword(true)}>
              Change password
            </Button>
          }
        />

        <SettingRow
          icon={ShieldCheck}
          title="Two-factor sign-in"
          badge={
            data ? (
              data.enabled ? (
                <Badge variant="solid" color="emerald" size="compact">
                  On
                </Badge>
              ) : (
                <Badge variant="dot" color="gray" size="compact">
                  Off
                </Badge>
              )
            ) : null
          }
          description={
            !data ? (
              <Skeleton className="mt-1 h-4 w-56" />
            ) : data.enabled ? (
              <span className={lowCodes ? 'text-warn' : undefined}>
                {data.recovery_codes_left} of 10 recovery codes remaining.
                {lowCodes ? ' Generate new codes before running out.' : ''}
              </span>
            ) : (
              'Protect sign-in with an authenticator app (1Password, Bitwarden, Google Authenticator, Aegis).'
            )
          }
          actions={
            !data ? (
              <Skeleton className="h-7 w-20" />
            ) : data.enabled ? (
              <>
                <Button variant="outline" size="sm" onClick={() => setStep({ kind: 'confirm', action: 'regenerate' })}>
                  New recovery codes
                </Button>
                <Button variant="outline" size="sm" onClick={() => setStep({ kind: 'confirm', action: 'disable' })}>
                  Turn off
                </Button>
              </>
            ) : (
              <Button size="sm" onClick={() => setStep({ kind: 'password' })}>
                Turn on
              </Button>
            )
          }
        />
      </div>

      <ChangePasswordDialog open={changingPassword} onOpenChange={setChangingPassword} />

      <Dialog open={step.kind !== 'idle'} onOpenChange={(open) => !open && step.kind !== 'codes' && close()}>
        <DialogContent size="sm">
          {step.kind === 'password' ? (
            <PasswordStep pending={setup.isPending} onSubmit={(password) => setup.mutate(password)} />
          ) : step.kind === 'scan' ? (
            <ScanStep enrollment={step.enrollment} pending={enable.isPending} onSubmit={(code) => enable.mutate(code)} />
          ) : step.kind === 'codes' ? (
            <CodesStep codes={step.codes} onDone={close} />
          ) : step.kind === 'confirm' ? (
            <ConfirmStep
              action={step.action}
              pending={disable.isPending || regenerate.isPending}
              onSubmit={(body) => (step.action === 'disable' ? disable.mutate(body) : regenerate.mutate(body))}
            />
          ) : null}
        </DialogContent>
      </Dialog>
    </Panel>
  )
}

function ChangePasswordDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (open: boolean) => void }) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent size="sm">
        {open ? <ChangePasswordForm onClose={() => onOpenChange(false)} /> : null}
      </DialogContent>
    </Dialog>
  )
}

function ChangePasswordForm({ onClose }: { onClose: () => void }) {
  const [mismatch, setMismatch] = useState(false)
  const change = useMutation({
    mutationFn: (body: Schemas['ChangePasswordRequest']) => unwrap(api.POST('/api/v1/auth/password', { body })),
    meta: { success: 'Password changed. Other sessions were signed out.' },
    onSuccess: onClose,
  })

  function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault()
    const form = new FormData(e.currentTarget)
    const next = String(form.get('new') ?? '')
    if (next !== String(form.get('confirm') ?? '')) {
      setMismatch(true)
      return
    }
    setMismatch(false)
    change.mutate({ current_password: String(form.get('current') ?? ''), new_password: next })
  }

  return (
    <form onSubmit={submit} className="contents">
      <DialogHeader>
        <DialogTitle>Change password</DialogTitle>
        <DialogDescription>Other browser sessions will be signed out. Access tokens keep working.</DialogDescription>
      </DialogHeader>
      <FieldGroup>
        <Field>
          <FieldLabel htmlFor="pw-current">Current password</FieldLabel>
          <Input id="pw-current" name="current" type="password" required autoFocus autoComplete="current-password" />
        </Field>
        <Field>
          <FieldLabel htmlFor="pw-new">New password</FieldLabel>
          <Input id="pw-new" name="new" type="password" required minLength={10} autoComplete="new-password" />
          <FieldDescription>At least 10 characters.</FieldDescription>
        </Field>
        <Field data-invalid={mismatch || undefined}>
          <FieldLabel htmlFor="pw-confirm">Repeat new password</FieldLabel>
          <Input
            id="pw-confirm"
            name="confirm"
            type="password"
            required
            autoComplete="new-password"
            aria-invalid={mismatch || undefined}
            onChange={() => mismatch && setMismatch(false)}
          />
          {mismatch ? <FieldError>The two new passwords do not match.</FieldError> : null}
        </Field>
      </FieldGroup>
      <DialogFooter>
        <Button type="button" variant="outline" onClick={onClose}>
          Cancel
        </Button>
        <Button type="submit" disabled={change.isPending}>
          {change.isPending ? <Spinner data-icon="inline-start" /> : null}
          Change password
        </Button>
      </DialogFooter>
    </form>
  )
}

const totpQuery = queryOptions({
  queryKey: ['account', 'totp'],
  queryFn: () => unwrap(api.GET('/api/v1/account/totp')),
})

type Step =
  | { kind: 'idle' }
  | { kind: 'password' }
  | { kind: 'scan'; enrollment: Schemas['TotpEnrollment'] }
  | { kind: 'codes'; codes: string[] }
  | { kind: 'confirm'; action: 'disable' | 'regenerate' }

function PasswordStep({ pending, onSubmit }: { pending: boolean; onSubmit: (password: string) => void }) {
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault()
        onSubmit(String(new FormData(e.currentTarget).get('password') ?? ''))
      }}
      className="contents"
    >
      <DialogHeader>
        <DialogTitle>Confirm your password</DialogTitle>
        <DialogDescription>Enter your current password to enable two-factor sign-in.</DialogDescription>
      </DialogHeader>
      <Field>
        <FieldLabel htmlFor="tfa-password">Password</FieldLabel>
        <Input id="tfa-password" name="password" type="password" required autoFocus autoComplete="current-password" />
      </Field>
      <DialogFooter>
        <Button type="submit" disabled={pending}>
          {pending ? <Spinner data-icon="inline-start" /> : null}
          Continue
        </Button>
      </DialogFooter>
    </form>
  )
}

function ScanStep({
  enrollment,
  pending,
  onSubmit,
}: {
  enrollment: Schemas['TotpEnrollment']
  pending: boolean
  onSubmit: (code: string) => void
}) {
  const src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(enrollment.qr_svg)}`
  const grouped = enrollment.secret.match(/.{1,4}/g)?.join(' ') ?? enrollment.secret
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault()
        onSubmit(String(new FormData(e.currentTarget).get('code') ?? '').trim())
      }}
      className="contents"
    >
      <DialogHeader>
        <DialogTitle>Scan with your authenticator</DialogTitle>
        <DialogDescription>Enter the six-digit code it generates to finish.</DialogDescription>
      </DialogHeader>
      <div className="flex flex-col items-center gap-3">
        <img src={src} alt="QR code for your authenticator app" width={192} height={192} className="rounded-lg border border-border bg-white p-1.5" />
        <details className="w-full rounded-lg border border-border/80 bg-surface/50 px-3 py-2 text-[13px]">
          <summary className="cursor-pointer font-medium text-muted-foreground hover:text-foreground">
            Can't scan? Enter the key by hand
          </summary>
          <div className="mt-2 flex items-center gap-2">
            <code className="min-w-0 flex-1 break-all rounded bg-card px-2 py-1 font-mono text-xs">{grouped}</code>
            <CopyButton value={enrollment.secret} label="Copy key" />
          </div>
        </details>
      </div>
      <Field>
        <FieldLabel htmlFor="tfa-code">Code</FieldLabel>
        <Input
          id="tfa-code"
          name="code"
          required
          autoFocus
          inputMode="numeric"
          autoComplete="one-time-code"
          pattern="[0-9 ]{6,7}"
          maxLength={7}
          placeholder="123 456"
          className="font-mono tracking-widest"
        />
      </Field>
      <DialogFooter>
        <Button type="submit" disabled={pending}>
          {pending ? <Spinner data-icon="inline-start" /> : null}
          Turn on
        </Button>
      </DialogFooter>
    </form>
  )
}

function CodesStep({ codes, onDone }: { codes: string[]; onDone: () => void }) {
  const text = codes.join('\n')
  return (
    <>
      <DialogHeader>
        <DialogTitle>Save your recovery codes</DialogTitle>
        <DialogDescription>
          Each code works once if you lose your authenticator. They are not shown again.
        </DialogDescription>
      </DialogHeader>
      <ul
        className="grid grid-cols-2 gap-x-6 gap-y-1.5 rounded-lg border border-border/80 bg-surface/60 p-3 font-mono text-[13px]"
        data-testid="recovery-codes"
      >
        {codes.map((c) => (
          <li key={c}>{c}</li>
        ))}
      </ul>
      <DialogFooter>
        <CopyButton value={text} label="Copy codes" />
        <Button
          variant="outline"
          onClick={() => {
            const url = URL.createObjectURL(new Blob([`Rig recovery codes\n\n${text}\n`], { type: 'text/plain' }))
            const a = Object.assign(document.createElement('a'), { href: url, download: 'rig-recovery-codes.txt' })
            a.click()
            URL.revokeObjectURL(url)
          }}
        >
          Download
        </Button>
        <Button onClick={onDone}>I saved them</Button>
      </DialogFooter>
    </>
  )
}

function ConfirmStep({
  action,
  pending,
  onSubmit,
}: {
  action: 'disable' | 'regenerate'
  pending: boolean
  onSubmit: (body: Schemas['TotpDisable']) => void
}) {
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault()
        const form = new FormData(e.currentTarget)
        onSubmit({ password: String(form.get('password') ?? ''), code: String(form.get('code') ?? '').trim() })
      }}
      className="contents"
    >
      <DialogHeader>
        <DialogTitle>{action === 'disable' ? 'Turn off two-factor sign-in' : 'Make new recovery codes'}</DialogTitle>
        <DialogDescription>
          {action === 'disable'
            ? 'Your password will be enough to sign in again. Other sessions are signed out.'
            : 'Existing recovery codes will stop working.'}
        </DialogDescription>
      </DialogHeader>
      <FieldGroup>
        <Field>
          <FieldLabel htmlFor="tfa-confirm-password">Password</FieldLabel>
          <Input id="tfa-confirm-password" name="password" type="password" required autoFocus autoComplete="current-password" />
        </Field>
        <Field>
          <FieldLabel htmlFor="tfa-confirm-code">Authenticator or recovery code</FieldLabel>
          <Input id="tfa-confirm-code" name="code" required autoComplete="one-time-code" className="font-mono" />
        </Field>
      </FieldGroup>
      <DialogFooter>
        <Button type="submit" variant={action === 'disable' ? 'destructive' : 'default'} disabled={pending}>
          {pending ? <Spinner data-icon="inline-start" /> : null}
          {action === 'disable' ? 'Turn off' : 'Make new codes'}
        </Button>
      </DialogFooter>
    </form>
  )
}
