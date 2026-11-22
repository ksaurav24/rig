import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { Check, Palette, Tag } from 'lucide-react'
import { useEffect, useState, type FormEvent } from 'react'
import { api, unwrap, type Schemas } from '@/api/client'
import { meQuery, setupStatusQuery } from '@/api/queries'
import { ImagePicker } from '@/components/app/image-picker'
import { OrgMark } from '@/components/app/logo'
import { Page, Panel } from '@/components/app/page'
import { Badge } from '@/components/fluid/badge'
import { Field, FieldDescription, FieldGroup, FieldLabel } from '@/components/fluid/field'
import { Input } from '@/components/fluid/input'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import {
  ACCENT_PRESETS,
  ENV_COLORS,
  isEnvBadgeColor,
  type AccentPreset,
  type EnvBadgeColor,
} from '@/lib/preferences'
import { useTheme } from '@/lib/theme'
import { cn } from '@/lib/utils'

export const Route = createFileRoute('/_app/settings/general')({
  staticData: { crumb: 'General' },
  component: GeneralSettingsPage,
})

function GeneralSettingsPage() {
  const queryClient = useQueryClient()
  const { data: me } = useQuery(meQuery)
  const { resolved } = useTheme()
  const org = me?.org

  const [name, setName] = useState(org?.name ?? '')
  const [envLabel, setEnvLabel] = useState(org?.env_label ?? '')
  const [envColor, setEnvColor] = useState<EnvBadgeColor>(
    isEnvBadgeColor(org?.env_color) ? org.env_color : 'emerald',
  )

  useEffect(() => {
    if (!org) return
    setName(org.name)
    setEnvLabel(org.env_label ?? '')
    setEnvColor(isEnvBadgeColor(org.env_color) ? org.env_color : 'emerald')
  }, [org?.name, org?.env_label, org?.env_color])

  function applyOrgUpdate(updated: Schemas['Organization']) {
    queryClient.setQueryData<Schemas['Me']>(meQuery.queryKey, (old) => (old ? { ...old, org: updated } : old))
    queryClient.setQueryData<Schemas['SetupStatus']>(setupStatusQuery.queryKey, (old) =>
      old
        ? {
            ...old,
            branding: {
              org_name: updated.name,
              logo_url: updated.logo_url ?? null,
              accent_color: updated.accent_color ?? 'emerald',
              env_label: updated.env_label ?? null,
              env_color: updated.env_color ?? null,
            },
          }
        : old,
    )
  }

  const updateOrg = useMutation({
    mutationFn: (body: Schemas['UpdateOrganizationRequest']) => unwrap(api.PATCH('/api/v1/org', { body })),
    meta: { success: 'Workspace settings saved' },
    onSuccess: applyOrgUpdate,
  })

  const uploadLogo = useMutation({
    mutationFn: (body: Schemas['UploadImageRequest']) => unwrap(api.POST('/api/v1/org/logo', { body })),
    meta: { success: 'Organization logo updated' },
    onSuccess: applyOrgUpdate,
  })

  const removeLogo = useMutation({
    mutationFn: () => unwrap(api.DELETE('/api/v1/org/logo')),
    meta: { success: 'Organization logo reset to default' },
    onSuccess: applyOrgUpdate,
  })

  function saveIdentity(e: FormEvent<HTMLFormElement>) {
    e.preventDefault()
    const trimmedName = name.trim()
    const trimmedEnv = envLabel.trim()
    if (!trimmedName) return
    updateOrg.mutate({
      name: trimmedName,
      env_label: trimmedEnv,
      env_color: trimmedEnv ? envColor : '',
    })
  }

  const nameChanged = org && name.trim() !== org.name
  const envChanged =
    org &&
    (envLabel.trim() !== (org.env_label ?? '') ||
      (envLabel.trim() !== '' && envColor !== (org.env_color ?? 'emerald')))

  const busy = updateOrg.isPending || uploadLogo.isPending || removeLogo.isPending

  return (
    <Page
      title="General"
      description="Customize your organization name, logo, environment badge, and workspace accent color."
      className="max-w-3xl"
    >
      <Panel
        title="Organization identity"
        description="Shown in the sidebar header, browser tab, favicon, and sign-in screen."
      >
        <div className="mb-5 flex flex-wrap items-center justify-between gap-4 rounded-xl border border-border/80 bg-surface/60 px-4 py-3">
          <div className="flex min-w-0 items-center gap-3">
            <OrgMark logoUrl={org?.logo_url} name={org?.name} className="size-10 rounded-xl" />
            <div className="min-w-0">
              <div className="flex flex-wrap items-center gap-2">
                <span className="truncate text-[15px] font-semibold text-foreground">
                  {name.trim() || org?.name || 'Organization'}
                </span>
                {envLabel.trim() ? (
                  <Badge variant="solid" color={envColor} size="compact">
                    {envLabel.trim()}
                  </Badge>
                ) : null}
              </div>
              <p className="text-[12px] text-muted-foreground">Live sidebar & login preview</p>
            </div>
          </div>
          <span className="font-mono text-[11.5px] text-muted-foreground">/{org?.slug}</span>
        </div>

        <form onSubmit={saveIdentity} className="space-y-5">
          <FieldGroup>
            <Field>
              <FieldLabel htmlFor="org-name">Organization name</FieldLabel>
              <Input
                id="org-name"
                value={name}
                onChange={(e) => setName(e.target.value)}
                required
                maxLength={80}
                placeholder="Acme Cloud"
              />
              <FieldDescription>Displayed in the sidebar header and sign-in page.</FieldDescription>
            </Field>

            <Field>
              <FieldLabel htmlFor="org-env">
                <span className="inline-flex items-center gap-1.5">
                  <Tag className="size-3.5 text-muted-foreground" />
                  Environment badge
                  <span className="font-normal text-muted-foreground">(optional)</span>
                </span>
              </FieldLabel>
              <div className="flex flex-col gap-3 sm:flex-row sm:items-center">
                <Input
                  id="org-env"
                  value={envLabel}
                  onChange={(e) => setEnvLabel(e.target.value)}
                  maxLength={24}
                  placeholder="e.g. Production, Staging, Homelab"
                  className="sm:max-w-xs"
                />
                <div className="flex flex-wrap items-center gap-1.5" role="radiogroup" aria-label="Environment badge color">
                  {ENV_COLORS.map((c) => {
                    const active = envColor === c.value
                    return (
                      <button
                        key={c.value}
                        type="button"
                        role="radio"
                        aria-checked={active}
                        title={c.label}
                        onClick={() => setEnvColor(c.value)}
                        className={cn(
                          'inline-flex h-7 items-center gap-1.5 rounded-full border px-2.5 text-[12px] transition-colors',
                          active
                            ? 'border-foreground/40 bg-accent font-medium text-foreground'
                            : 'border-border/80 bg-card text-muted-foreground hover:text-foreground',
                        )}
                      >
                        <span className={cn('size-2 rounded-full', c.dot)} />
                        {c.label}
                      </button>
                    )
                  })}
                </div>
              </div>
              <FieldDescription>
                Leave blank to keep the sidebar minimal. Useful when managing multiple Rig servers.
              </FieldDescription>
            </Field>
          </FieldGroup>

          <div className="flex items-center gap-2">
            <Button type="submit" size="sm" disabled={busy || (!nameChanged && !envChanged)}>
              {updateOrg.isPending ? <Spinner data-icon="inline-start" /> : null}
              Save changes
            </Button>
            {envLabel.trim() ? (
              <Button
                type="button"
                variant="ghost"
                size="sm"
                disabled={busy}
                onClick={() => {
                  setEnvLabel('')
                  if (org?.env_label) {
                    updateOrg.mutate({ env_label: '', env_color: '' })
                  }
                }}
              >
                Clear badge
              </Button>
            ) : null}
          </div>
        </form>
      </Panel>

      <Panel
        title="Organization logo"
        description="Upload a logo file or link to a hosted image. Also used as the browser favicon and on the sign-in screen."
      >
        <ImagePicker
          currentUrl={org?.logo_url}
          pending={uploadLogo.isPending || removeLogo.isPending || updateOrg.isPending}
          onUpload={(payload) => uploadLogo.mutate(payload)}
          onSetUrl={(logo_url) => updateOrg.mutate({ logo_url })}
          onRemove={() => removeLogo.mutate()}
          removeLabel="Reset to Rig mark"
        />
      </Panel>

      <Panel
        title="Brand accent color"
        description="Sets the default workspace accent color for everyone in this organization and on the sign-in screen. Members can still pick a personal override in Account settings."
      >
        <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
          {ACCENT_PRESETS.map((preset) => {
            const active = (org?.accent_color ?? 'emerald') === preset.value
            const swatch = resolved === 'dark' ? preset.swatchDark : preset.swatchLight
            return (
              <button
                key={preset.value}
                type="button"
                disabled={updateOrg.isPending}
                aria-pressed={active}
                onClick={() => updateOrg.mutate({ accent_color: preset.value as AccentPreset })}
                className={cn(
                  'flex items-center gap-2.5 rounded-xl border px-3 py-2.5 text-left transition-colors',
                  active
                    ? 'border-primary bg-primary/[0.06] text-foreground shadow-[0_0_0_1px_var(--primary)]'
                    : 'border-border/80 bg-card text-foreground hover:bg-hover',
                )}
              >
                <span
                  className="grid size-5 shrink-0 place-items-center rounded-full shadow-xs"
                  style={{ backgroundColor: swatch }}
                >
                  {active ? <Check className="size-3 text-white mix-blend-difference" /> : null}
                </span>
                <span className="truncate text-[13px] font-medium">{preset.label}</span>
              </button>
            )
          })}
        </div>
        <p className="mt-3 flex items-center gap-1.5 text-[12px] text-muted-foreground">
          <Palette className="size-3.5" />
          Curated OKLCH palettes keep text and controls readable in both light and dark modes.
        </p>
      </Panel>
    </Page>
  )
}
