import { useQuery, useQueryClient } from '@tanstack/react-query'
import { save as saveDialog } from '@tauri-apps/plugin-dialog'
import {
  CheckCircle2,
  Copy,
  Download,
  KeyRound,
  Lock,
  Pencil,
  RefreshCw,
  ShieldAlert,
  ShieldCheck,
  User as UserIcon,
} from 'lucide-react'
import { useEffect, useState, type FormEvent } from 'react'
import { toast } from 'sonner'
import { Panel } from '@rig/ui/app/page'
import { Badge } from '@rig/ui/badge'
import { Banner, BannerDescription, BannerTitle } from '@rig/ui/banner'
import { Button } from '@rig/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@rig/ui/dialog'
import { Field, FieldDescription, FieldLabel } from '@rig/ui/field'
import { Input } from '@rig/ui/input'
import { ImagePicker } from './ImagePicker'
import { UserAvatar } from './MediaImage'
import { relativeTime } from '@/lib/format'
import { desktopApi } from '@/lib/ipc'
import type { Me, TotpEnrollment } from '@/lib/types'

interface AccountSettingsPanelProps {
  serverId: string
  serverName: string
  me: Me | null | undefined
  onChanged: () => void
}

export function AccountSettingsPanel({
  serverId,
  serverName,
  me,
  onChanged,
}: AccountSettingsPanelProps) {
  const qc = useQueryClient()
  const [profileOpen, setProfileOpen] = useState(false)
  const [passwordOpen, setPasswordOpen] = useState(false)
  const [totpDialog, setTotpDialog] = useState<'enable' | 'disable' | 'regenerate' | null>(null)

  const totpQuery = useQuery({
    queryKey: ['desktop', serverId, 'totp-status'],
    queryFn: () => desktopApi.getTotpStatus(serverId),
    enabled: Boolean(me),
  })

  const totp = totpQuery.data

  function refreshAll() {
    void totpQuery.refetch()
    void qc.invalidateQueries({ queryKey: ['desktop', serverId, 'settings'] })
    onChanged()
  }

  return (
    <div className="space-y-5">
      {/* Profile & Avatar */}
      <Panel
        title={
          <>
            <UserIcon className="size-4 text-primary" />
            Profile &amp; Avatar
          </>
        }
        description="Your display name and avatar shown in audit logs and team views on this server."
        actions={
          <Button
            type="button"
            variant="secondary"
            size="compact"
            leadingIcon={Pencil}
            disabled={!me}
            onClick={() => setProfileOpen(true)}
          >
            Edit Profile
          </Button>
        }
      >
        {me ? (
          <div className="flex flex-wrap items-center gap-4">
            <UserAvatar
              serverId={serverId}
              name={me.user.name}
              avatarUrl={me.user.avatar_url}
              className="size-14 text-[18px]"
            />
            <div className="min-w-0 flex-1">
              <div className="flex flex-wrap items-center gap-2">
                <span className="truncate text-[16px] font-semibold text-foreground">
                  {me.user.name}
                </span>
                <Badge variant="solid" color="emerald" size="compact" className="capitalize">
                  {me.role}
                </Badge>
              </div>
              <div className="mt-0.5 truncate font-mono text-[13px] text-muted-foreground">
                {me.user.email}
              </div>
            </div>
          </div>
        ) : (
          <p className="text-[13px] text-muted-foreground">Loading account details…</p>
        )}
      </Panel>

      {/* Password */}
      <Panel
        title={
          <>
            <Lock className="size-4 text-primary" />
            Password
          </>
        }
        description="Changing your password signs out active browser sessions while keeping paired desktop devices signed in."
        actions={
          <Button
            type="button"
            variant="tertiary"
            size="compact"
            leadingIcon={KeyRound}
            disabled={!me}
            onClick={() => setPasswordOpen(true)}
          >
            Change Password
          </Button>
        }
      >
        <p className="text-[13px] text-muted-foreground">
          Use a unique passphrase of at least 10 characters for{' '}
          <span className="font-mono text-foreground">{me?.user.email ?? 'this account'}</span>.
        </p>
      </Panel>

      {/* Two-Factor Authentication (TOTP) */}
      <Panel
        title={
          <>
            {totp?.enabled ? (
              <ShieldCheck className="size-4 text-ok" />
            ) : (
              <ShieldAlert className="size-4 text-warn" />
            )}
            Two-Factor Authentication (TOTP)
          </>
        }
        description="Protect sign-ins with a 6-digit authenticator app code and single-use recovery codes."
        actions={
          totp?.enabled ? (
            <div className="flex flex-wrap items-center gap-2">
              <Button
                type="button"
                variant="tertiary"
                size="compact"
                leadingIcon={RefreshCw}
                onClick={() => setTotpDialog('regenerate')}
              >
                New Recovery Codes
              </Button>
              <Button
                type="button"
                variant="destructive"
                size="compact"
                onClick={() => setTotpDialog('disable')}
              >
                Turn Off 2FA
              </Button>
            </div>
          ) : (
            <Button
              type="button"
              variant="primary"
              size="compact"
              leadingIcon={ShieldCheck}
              disabled={!me || totpQuery.isPending}
              onClick={() => setTotpDialog('enable')}
            >
              Enable 2FA
            </Button>
          )
        }
      >
        {totpQuery.isPending ? (
          <p className="text-[13px] text-muted-foreground">Checking 2FA status…</p>
        ) : totp?.enabled ? (
          <div className="space-y-3">
            <div className="flex flex-wrap items-center gap-2.5 text-[13.5px]">
              <Badge variant="dot" color="emerald" size="compact">
                Enabled
              </Badge>
              {totp.enabled_at && (
                <span className="text-[12.5px] text-muted-foreground">
                  Active since {relativeTime(totp.enabled_at)}
                </span>
              )}
              <span className="text-[12.5px] text-muted-foreground">·</span>
              <span className="font-mono text-[12.5px] text-foreground">
                {totp.recovery_codes_left} recovery{' '}
                {totp.recovery_codes_left === 1 ? 'code' : 'codes'} remaining
              </span>
            </div>
            {totp.recovery_codes_left <= 2 && (
              <Banner status="warning" contrast="high" size="compact">
                <BannerDescription>
                  You are running low on recovery codes. Generate a fresh set so you do not get
                  locked out if you lose your authenticator device.
                </BannerDescription>
              </Banner>
            )}
          </div>
        ) : (
          <p className="text-[13px] text-muted-foreground">
            Two-factor sign-in is currently off. Enable TOTP to require an authenticator code when
            signing in on browsers or pairing new devices with a password.
          </p>
        )}
      </Panel>

      {/* Edit Profile Dialog */}
      {me && (
        <EditProfileDialog
          open={profileOpen}
          serverId={serverId}
          me={me}
          onClose={() => setProfileOpen(false)}
          onSaved={refreshAll}
        />
      )}

      {/* Change Password Dialog */}
      <ChangePasswordDialog
        open={passwordOpen}
        serverId={serverId}
        onClose={() => setPasswordOpen(false)}
      />

      {/* Enable TOTP Wizard Dialog */}
      <EnableTotpDialog
        open={totpDialog === 'enable'}
        serverId={serverId}
        serverName={serverName}
        onClose={() => setTotpDialog(null)}
        onComplete={refreshAll}
      />

      {/* Disable or Regenerate TOTP Dialog */}
      <ManageTotpDialog
        mode={totpDialog === 'disable' || totpDialog === 'regenerate' ? totpDialog : null}
        serverId={serverId}
        serverName={serverName}
        onClose={() => setTotpDialog(null)}
        onComplete={refreshAll}
      />
    </div>
  )
}

function EditProfileDialog({
  open,
  serverId,
  me,
  onClose,
  onSaved,
}: {
  open: boolean
  serverId: string
  me: Me
  onClose: () => void
  onSaved: () => void
}) {
  const [name, setName] = useState(me.user.name)
  const [avatarUrl, setAvatarUrl] = useState<string | null | undefined>(me.user.avatar_url)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (!open) return
    setName(me.user.name)
    setAvatarUrl(me.user.avatar_url)
    setError(null)
  }, [open, me])

  async function saveName(e: FormEvent) {
    e.preventDefault()
    if (!name.trim()) return
    setSaving(true)
    setError(null)
    try {
      await desktopApi.updateProfile(serverId, { name: name.trim() })
      toast.success('Profile updated')
      onSaved()
      onClose()
    } catch (err) {
      setError(String(err))
    } finally {
      setSaving(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={(next) => !next && onClose()}>
      <DialogContent size="lg">
        <DialogHeader>
          <DialogTitle>Edit Profile</DialogTitle>
          <DialogDescription>
            Update your display name or pick a preset, uploaded photo, or image URL.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-5">
          {error && (
            <Banner role="alert" status="error" contrast="high" size="compact">
              <BannerDescription>{error}</BannerDescription>
            </Banner>
          )}

          <div className="flex items-center gap-4 rounded-xl border border-border/80 bg-surface/60 p-3.5">
            <UserAvatar
              serverId={serverId}
              name={name || me.user.name}
              avatarUrl={avatarUrl}
              className="size-14 text-[18px]"
            />
            <div className="min-w-0">
              <div className="truncate text-[14.5px] font-semibold text-foreground">
                {name || me.user.name}
              </div>
              <div className="truncate font-mono text-[12px] text-muted-foreground">
                {me.user.email}
              </div>
            </div>
          </div>

          <ImagePicker
            presets
            currentUrl={avatarUrl}
            busy={saving}
            onUpload={async (contentType, dataBase64) => {
              const updated = await desktopApi.uploadAvatar(serverId, {
                content_type: contentType,
                data_base64: dataBase64,
              })
              setAvatarUrl(updated.avatar_url)
              toast.success('Avatar uploaded')
              onSaved()
            }}
            onSelectUrl={async (url) => {
              const updated = await desktopApi.updateProfile(serverId, { avatar_url: url })
              setAvatarUrl(updated.avatar_url)
              toast.success('Avatar updated')
              onSaved()
            }}
            onReset={async () => {
              const updated = await desktopApi.deleteAvatar(serverId)
              setAvatarUrl(updated.avatar_url)
              toast.success('Avatar reset to initials')
              onSaved()
            }}
          />

          <form onSubmit={saveName} className="space-y-4 border-t border-border/80 pt-4">
            <Field>
              <FieldLabel htmlFor="acct-name">Display Name</FieldLabel>
              <Input
                id="acct-name"
                type="text"
                required
                maxLength={80}
                value={name}
                onChange={(e) => setName(e.target.value)}
              />
            </Field>
            <DialogFooter>
              <Button type="button" variant="ghost" onClick={onClose}>
                Close
              </Button>
              <Button type="submit" variant="primary" loading={saving} disabled={!name.trim()}>
                Save Name
              </Button>
            </DialogFooter>
          </form>
        </div>
      </DialogContent>
    </Dialog>
  )
}

function ChangePasswordDialog({
  open,
  serverId,
  onClose,
}: {
  open: boolean
  serverId: string
  onClose: () => void
}) {
  const [currentPassword, setCurrentPassword] = useState('')
  const [newPassword, setNewPassword] = useState('')
  const [confirmPassword, setConfirmPassword] = useState('')
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (!open) return
    setCurrentPassword('')
    setNewPassword('')
    setConfirmPassword('')
    setError(null)
  }, [open])

  async function handleSubmit(e: FormEvent) {
    e.preventDefault()
    if (newPassword.length < 10) {
      setError('New password must be at least 10 characters.')
      return
    }
    if (newPassword !== confirmPassword) {
      setError('New passwords do not match.')
      return
    }
    setSaving(true)
    setError(null)
    try {
      await desktopApi.changePassword(serverId, {
        current_password: currentPassword,
        new_password: newPassword,
      })
      toast.success('Password changed')
      onClose()
    } catch (err) {
      setError(String(err))
    } finally {
      setSaving(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={(next) => !next && onClose()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Change Password</DialogTitle>
          <DialogDescription>
            Enter your current password and choose a new password of at least 10 characters.
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="space-y-4">
          {error && (
            <Banner role="alert" status="error" contrast="high" size="compact">
              <BannerDescription>{error}</BannerDescription>
            </Banner>
          )}
          <Field>
            <FieldLabel htmlFor="pw-current">Current Password</FieldLabel>
            <Input
              id="pw-current"
              type="password"
              required
              autoFocus
              value={currentPassword}
              onChange={(e) => setCurrentPassword(e.target.value)}
            />
          </Field>
          <Field>
            <FieldLabel htmlFor="pw-new">New Password</FieldLabel>
            <Input
              id="pw-new"
              type="password"
              required
              minLength={10}
              value={newPassword}
              onChange={(e) => setNewPassword(e.target.value)}
            />
            <FieldDescription>Minimum 10 characters.</FieldDescription>
          </Field>
          <Field>
            <FieldLabel htmlFor="pw-confirm">Confirm New Password</FieldLabel>
            <Input
              id="pw-confirm"
              type="password"
              required
              minLength={10}
              value={confirmPassword}
              onChange={(e) => setConfirmPassword(e.target.value)}
            />
          </Field>
          <DialogFooter>
            <Button type="button" variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit" variant="primary" loading={saving}>
              Update Password
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}

function RecoveryCodesBlock({
  codes,
  serverName,
}: {
  codes: string[]
  serverName: string
}) {
  const joined = codes.join('\n')

  async function saveToFile() {
    const slug = serverName.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '') || 'rig'
    const dest = await saveDialog({
      defaultPath: `${slug}-recovery-codes.txt`,
      filters: [{ name: 'Text File', extensions: ['txt'] }],
      title: 'Save Rig Recovery Codes',
    })
    if (!dest) return
    try {
      const content = `# Rig 2FA Recovery Codes (${serverName})\n# Each code can be used once.\n\n${joined}\n`
      await desktopApi.saveTextFile(dest, content)
      toast.success(`Saved recovery codes to ${dest}`)
    } catch (err) {
      toast.error(String(err))
    }
  }

  return (
    <div className="space-y-3">
      <Banner status="warning" contrast="high" size="compact">
        <BannerTitle>Save these recovery codes now</BannerTitle>
        <BannerDescription>
          Each code works once if you lose access to your authenticator app. They will not be shown
          again.
        </BannerDescription>
      </Banner>
      <div className="grid grid-cols-2 gap-2 rounded-xl border border-border/80 bg-surface p-3.5 font-mono text-[13px] text-foreground">
        {codes.map((c) => (
          <div key={c} className="rounded bg-card px-2.5 py-1.5 text-center select-all">
            {c}
          </div>
        ))}
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <Button
          type="button"
          variant="secondary"
          size="compact"
          leadingIcon={Copy}
          onClick={() => {
            void navigator.clipboard.writeText(joined)
            toast.success('Copied all recovery codes')
          }}
        >
          Copy All
        </Button>
        <Button
          type="button"
          variant="tertiary"
          size="compact"
          leadingIcon={Download}
          onClick={() => void saveToFile()}
        >
          Save as .txt…
        </Button>
      </div>
    </div>
  )
}

function EnableTotpDialog({
  open,
  serverId,
  serverName,
  onClose,
  onComplete,
}: {
  open: boolean
  serverId: string
  serverName: string
  onClose: () => void
  onComplete: () => void
}) {
  const [step, setStep] = useState<'password' | 'scan' | 'codes'>('password')
  const [password, setPassword] = useState('')
  const [enrollment, setEnrollment] = useState<TotpEnrollment | null>(null)
  const [code, setCode] = useState('')
  const [codes, setCodes] = useState<string[]>([])
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (!open) return
    setStep('password')
    setPassword('')
    setEnrollment(null)
    setCode('')
    setCodes([])
    setError(null)
  }, [open])

  async function startSetup(e: FormEvent) {
    e.preventDefault()
    setBusy(true)
    setError(null)
    try {
      const res = await desktopApi.totpSetup(serverId, password)
      setEnrollment(res)
      setStep('scan')
    } catch (err) {
      setError(String(err))
    } finally {
      setBusy(false)
    }
  }

  async function verifyCode(e: FormEvent) {
    e.preventDefault()
    setBusy(true)
    setError(null)
    try {
      const res = await desktopApi.totpEnable(serverId, code.trim())
      setCodes(res.codes)
      setStep('codes')
      toast.success('Two-factor authentication enabled')
      onComplete()
    } catch (err) {
      setError(String(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={(next) => !next && onClose()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <ShieldCheck className="size-4 text-primary" />
            {step === 'password'
              ? 'Confirm Your Password'
              : step === 'scan'
                ? 'Scan QR Code'
                : 'Two-Factor Enabled'}
          </DialogTitle>
          <DialogDescription>
            {step === 'password'
              ? 'Confirm your password to generate a new authenticator secret.'
              : step === 'scan'
                ? 'Scan this QR code in 1Password, Authy, Bitwarden, or Google Authenticator, then enter the 6-digit code.'
                : 'Store your backup recovery codes somewhere safe before closing.'}
          </DialogDescription>
        </DialogHeader>

        {error && (
          <Banner role="alert" status="error" contrast="high" size="compact">
            <BannerDescription>{error}</BannerDescription>
          </Banner>
        )}

        {step === 'password' && (
          <form onSubmit={startSetup} className="space-y-4">
            <Field>
              <FieldLabel htmlFor="totp-pw">Account Password</FieldLabel>
              <Input
                id="totp-pw"
                type="password"
                required
                autoFocus
                value={password}
                onChange={(e) => setPassword(e.target.value)}
              />
            </Field>
            <DialogFooter>
              <Button type="button" variant="ghost" onClick={onClose}>
                Cancel
              </Button>
              <Button type="submit" variant="primary" loading={busy} disabled={!password}>
                Continue
              </Button>
            </DialogFooter>
          </form>
        )}

        {step === 'scan' && enrollment && (
          <form onSubmit={verifyCode} className="space-y-4">
            <div className="flex flex-col items-center gap-3 rounded-xl border border-border/80 bg-surface/60 p-4">
              <div
                className="size-44 overflow-hidden rounded-lg bg-white p-2.5 shadow-sm [&>svg]:size-full"
                dangerouslySetInnerHTML={{ __html: enrollment.qr_svg }}
              />
              <div className="flex max-w-full items-center gap-1.5 rounded-md border border-border bg-card px-2.5 py-1 font-mono text-[12px]">
                <span className="truncate select-all">{enrollment.secret}</span>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-compact"
                  aria-label="Copy manual key"
                  onClick={() => {
                    void navigator.clipboard.writeText(enrollment.secret)
                    toast.success('Copied setup key')
                  }}
                >
                  <Copy className="size-3.5" />
                </Button>
              </div>
            </div>
            <Field>
              <FieldLabel htmlFor="totp-verify-code">6-Digit Authenticator Code</FieldLabel>
              <Input
                id="totp-verify-code"
                type="text"
                inputMode="numeric"
                autoComplete="one-time-code"
                required
                autoFocus
                value={code}
                onChange={(e) => setCode(e.target.value)}
                placeholder="123456"
                className="font-mono tracking-widest"
              />
            </Field>
            <DialogFooter>
              <Button type="button" variant="ghost" onClick={onClose}>
                Cancel
              </Button>
              <Button type="submit" variant="primary" loading={busy} disabled={code.trim().length < 6}>
                Verify &amp; Enable
              </Button>
            </DialogFooter>
          </form>
        )}

        {step === 'codes' && (
          <div className="space-y-4">
            <RecoveryCodesBlock codes={codes} serverName={serverName} />
            <DialogFooter>
              <Button type="button" variant="primary" leadingIcon={CheckCircle2} onClick={onClose}>
                I&rsquo;ve Saved These Codes
              </Button>
            </DialogFooter>
          </div>
        )}
      </DialogContent>
    </Dialog>
  )
}

function ManageTotpDialog({
  mode,
  serverId,
  serverName,
  onClose,
  onComplete,
}: {
  mode: 'disable' | 'regenerate' | null
  serverId: string
  serverName: string
  onClose: () => void
  onComplete: () => void
}) {
  const [password, setPassword] = useState('')
  const [code, setCode] = useState('')
  const [newCodes, setNewCodes] = useState<string[] | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (!mode) return
    setPassword('')
    setCode('')
    setNewCodes(null)
    setError(null)
  }, [mode])

  async function handleSubmit(e: FormEvent) {
    e.preventDefault()
    if (!mode) return
    setBusy(true)
    setError(null)
    try {
      if (mode === 'disable') {
        await desktopApi.totpDisable(serverId, password, code.trim())
        toast.success('Two-factor authentication turned off')
        onComplete()
        onClose()
      } else {
        const res = await desktopApi.totpRegenerateRecoveryCodes(serverId, password, code.trim())
        setNewCodes(res.codes)
        toast.success('Generated new recovery codes')
        onComplete()
      }
    } catch (err) {
      setError(String(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog open={Boolean(mode)} onOpenChange={(next) => !next && onClose()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>
            {mode === 'disable' ? 'Turn Off Two-Factor Authentication' : 'Regenerate Recovery Codes'}
          </DialogTitle>
          <DialogDescription>
            {mode === 'disable'
              ? 'Confirm your password and a current authenticator or recovery code to disable 2FA.'
              : 'Your previous recovery codes will stop working immediately once new ones are generated.'}
          </DialogDescription>
        </DialogHeader>

        {newCodes ? (
          <div className="space-y-4">
            <RecoveryCodesBlock codes={newCodes} serverName={serverName} />
            <DialogFooter>
              <Button type="button" variant="primary" onClick={onClose}>
                Done
              </Button>
            </DialogFooter>
          </div>
        ) : (
          <form onSubmit={handleSubmit} className="space-y-4">
            {error && (
              <Banner role="alert" status="error" contrast="high" size="compact">
                <BannerDescription>{error}</BannerDescription>
              </Banner>
            )}
            <Field>
              <FieldLabel htmlFor="totp-mgr-pw">Account Password</FieldLabel>
              <Input
                id="totp-mgr-pw"
                type="password"
                required
                autoFocus
                value={password}
                onChange={(e) => setPassword(e.target.value)}
              />
            </Field>
            <Field>
              <FieldLabel htmlFor="totp-mgr-code">Authenticator or Recovery Code</FieldLabel>
              <Input
                id="totp-mgr-code"
                type="text"
                required
                value={code}
                onChange={(e) => setCode(e.target.value)}
                placeholder="123456 or ABCD-EFGH"
                className="font-mono"
              />
            </Field>
            <DialogFooter>
              <Button type="button" variant="ghost" onClick={onClose}>
                Cancel
              </Button>
              <Button
                type="submit"
                variant={mode === 'disable' ? 'destructive' : 'primary'}
                loading={busy}
                disabled={!password || !code.trim()}
              >
                {mode === 'disable' ? 'Turn Off 2FA' : 'Generate New Codes'}
              </Button>
            </DialogFooter>
          </form>
        )}
      </DialogContent>
    </Dialog>
  )
}
