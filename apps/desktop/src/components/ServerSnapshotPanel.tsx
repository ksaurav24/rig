import { open as openDialog, save as saveDialog } from '@tauri-apps/plugin-dialog'
import {
  AlertTriangle,
  Archive,
  BellRing,
  CheckCircle2,
  Clock,
  Download,
  Eye,
  EyeOff,
  FileArchive,
  FolderOpen,
  HardDriveDownload,
  Lock,
  RotateCcw,
  Server,
  ShieldCheck,
  Upload,
} from 'lucide-react'
import { useEffect, useMemo, useState, type FormEvent } from 'react'
import { toast } from 'sonner'
import { Panel } from '@rig/ui/app/page'
import { Badge } from '@rig/ui/badge'
import { Banner, BannerDescription, BannerTitle } from '@rig/ui/banner'
import { Button } from '@rig/ui/button'
import { Checkbox } from '@rig/ui/checkbox'
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
import { Select, SelectContent, SelectItem, SelectTrigger } from '@rig/ui/select'
import { Switch } from '@rig/ui/switch'
import { formatBytes, relativeTime } from '@/lib/format'
import { desktopApi } from '@/lib/ipc'
import type {
  BackupReminder,
  ServerView,
  SnapshotInspectResult,
  SnapshotRestoreMode,
  SnapshotRestoreReport,
} from '@/lib/types'
import { cn } from '@rig/ui/lib/utils'

interface ServerSnapshotPanelProps {
  server: ServerView
  servers: ServerView[]
  onServersChanged: () => void
  onRefreshServer: () => void
}

const REMINDER_PRESETS = [
  { days: 1, label: 'Daily' },
  { days: 7, label: 'Weekly' },
  { days: 14, label: 'Every 2 weeks' },
  { days: 30, label: 'Monthly' },
]

export function isBackupReminderDue(rem?: BackupReminder | null): boolean {
  if (!rem || !rem.enabled) return false
  const now = Date.now()
  if (rem.snoozed_until) {
    const snoozedMs = Date.parse(rem.snoozed_until)
    if (!Number.isNaN(snoozedMs) && snoozedMs > now) return false
  }
  if (!rem.last_snapshot_at) return true
  const lastMs = Date.parse(rem.last_snapshot_at)
  if (Number.isNaN(lastMs)) return true
  const days = Math.min(365, Math.max(1, rem.interval_days || 7))
  return now >= lastMs + days * 24 * 60 * 60 * 1000
}

export function ServerSnapshotPanel({
  server,
  servers,
  onServersChanged,
  onRefreshServer,
}: ServerSnapshotPanelProps) {
  const reminder = server.backup_reminder
  const due = isBackupReminderDue(reminder)

  // Reminder state
  const [remEnabled, setRemEnabled] = useState(reminder?.enabled ?? false)
  const [remDays, setRemDays] = useState(String(reminder?.interval_days ?? 7))
  const [savingRem, setSavingRem] = useState(false)

  // Export snapshot state
  const [includeDbs, setIncludeDbs] = useState(true)
  const [includeAlerts, setIncludeAlerts] = useState(true)
  const [includeGithub, setIncludeGithub] = useState(false)
  const [encrypt, setEncrypt] = useState(true)
  const [exportPass, setExportPass] = useState('')
  const [exportPassConfirm, setExportPassConfirm] = useState('')
  const [showExportPass, setShowExportPass] = useState(false)
  const [exporting, setExporting] = useState(false)

  // Restore snapshot state
  const [restorePath, setRestorePath] = useState('')
  const [targetServerId, setTargetServerId] = useState(server.id)
  const [restorePass, setRestorePass] = useState('')
  const [showRestorePass, setShowRestorePass] = useState(false)
  const [inspecting, setInspecting] = useState(false)
  const [inspectError, setInspectError] = useState<string | null>(null)
  const [preview, setPreview] = useState<SnapshotInspectResult | null>(null)
  const [restoreMode, setRestoreMode] = useState<SnapshotRestoreMode>('merge')
  const [restoreBranding, setRestoreBranding] = useState(true)
  const [restoreAlerts, setRestoreAlerts] = useState(true)
  const [restoreGithub, setRestoreGithub] = useState(false)
  const [startDatabases, setStartDatabases] = useState(true)
  const [deployRunningApps, setDeployRunningApps] = useState(true)
  const [confirmOpen, setConfirmOpen] = useState(false)
  const [replaceConfirmText, setReplaceConfirmText] = useState('')
  const [restoring, setRestoring] = useState(false)
  const [report, setReport] = useState<SnapshotRestoreReport | null>(null)

  useEffect(() => {
    setRemEnabled(reminder?.enabled ?? false)
    setRemDays(String(reminder?.interval_days ?? 7))
  }, [reminder?.enabled, reminder?.interval_days, server.id])

  useEffect(() => {
    setTargetServerId(server.id)
    setPreview(null)
    setInspectError(null)
    setReport(null)
  }, [server.id])

  const targetServer = useMemo(
    () => servers.find((s) => s.id === targetServerId) ?? server,
    [servers, targetServerId, server],
  )

  const conflictCount = useMemo(() => {
    if (!preview) return 0
    return (
      preview.projects.filter((p) => p.exists_on_target).length +
      preview.apps.filter((a) => a.exists_on_target).length +
      preview.databases.filter((d) => d.exists_on_target).length
    )
  }, [preview])

  async function saveReminder(nextEnabled: boolean, nextDaysRaw: string) {
    const parsed = Math.min(365, Math.max(1, Number.parseInt(nextDaysRaw, 10) || 7))
    setSavingRem(true)
    try {
      await desktopApi.setBackupReminder(server.id, nextEnabled, parsed)
      setRemEnabled(nextEnabled)
      setRemDays(String(parsed))
      toast.success(
        nextEnabled
          ? `Backup reminder set for every ${parsed} ${parsed === 1 ? 'day' : 'days'}`
          : 'Backup reminder turned off',
      )
      onServersChanged()
    } catch (e) {
      toast.error(String(e))
    } finally {
      setSavingRem(false)
    }
  }

  async function handleSnooze() {
    setSavingRem(true)
    try {
      await desktopApi.snoozeBackupReminder(server.id, 24)
      toast.success('Snoozed backup reminder for 24 hours')
      onServersChanged()
    } catch (e) {
      toast.error(String(e))
    } finally {
      setSavingRem(false)
    }
  }

  async function handleExport(e: FormEvent) {
    e.preventDefault()
    if (encrypt) {
      if (exportPass.length < 8) {
        toast.error('Use a passphrase of at least 8 characters, or disable encryption.')
        return
      }
      if (exportPass !== exportPassConfirm) {
        toast.error('Passphrases do not match.')
        return
      }
    }

    const slug =
      server.name
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, '-')
        .replace(/^-|-$/g, '') || 'rig-server'
    const stamp = new Date().toISOString().slice(0, 10)
    const ext = encrypt ? 'rig-snapshot.tar.zst.enc' : 'rig-snapshot.tar.zst'
    const dest = await saveDialog({
      title: `Save Full Snapshot of ${server.name}`,
      defaultPath: `${slug}-${stamp}.${ext}`,
      filters: [
        {
          name: encrypt ? 'Encrypted Rig Snapshot' : 'Rig Snapshot Archive',
          extensions: encrypt ? ['enc'] : ['zst'],
        },
      ],
    })
    if (!dest) return

    setExporting(true)
    try {
      const res = await desktopApi.exportServerSnapshot(
        server.id,
        {
          include_database_dumps: includeDbs,
          include_alerts: includeAlerts,
          include_github: includeGithub,
          passphrase: encrypt ? exportPass : null,
        },
        dest,
      )
      toast.success(`Saved server snapshot (${formatBytes(res.bytes)})`, {
        description: res.path,
      })
      setExportPass('')
      setExportPassConfirm('')
      onServersChanged()
    } catch (err) {
      toast.error(String(err))
    } finally {
      setExporting(false)
    }
  }

  async function runInspect(path: string, srvId: string, pass: string) {
    if (!path.trim()) return
    setInspecting(true)
    setInspectError(null)
    setReport(null)
    try {
      const res = await desktopApi.inspectServerSnapshot(srvId, path, pass || null)
      setPreview(res)
      setRestoreGithub(res.has_github_app)
    } catch (err) {
      setPreview(null)
      setInspectError(String(err))
    } finally {
      setInspecting(false)
    }
  }

  async function pickSnapshotFile() {
    const picked = await openDialog({
      multiple: false,
      title: 'Select Rig Server Snapshot Archive',
      filters: [
        { name: 'Rig Snapshots', extensions: ['enc', 'zst'] },
        { name: 'All Files', extensions: ['*'] },
      ],
    })
    if (typeof picked !== 'string') return
    setRestorePath(picked)
    setPreview(null)
    setInspectError(null)
    // Automatically dry-run inspect if unencrypted or if user already typed passphrase.
    if (!picked.endsWith('.enc') || restorePass.trim()) {
      await runInspect(picked, targetServerId, restorePass)
    }
  }

  async function executeRestore() {
    if (!restorePath || !preview) return
    setRestoring(true)
    try {
      const res = await desktopApi.restoreServerSnapshot(
        targetServerId,
        restorePath,
        {
          mode: restoreMode,
          restore_branding: restoreBranding,
          restore_alerts: restoreAlerts,
          restore_github: restoreGithub,
          start_databases: startDatabases,
          deploy_running_apps: deployRunningApps,
        },
        restorePass || null,
      )
      setReport(res)
      setConfirmOpen(false)
      setReplaceConfirmText('')
      toast.success(`Restored snapshot onto ${targetServer.name}`)
      onRefreshServer()
    } catch (err) {
      toast.error(String(err))
    } finally {
      setRestoring(false)
    }
  }

  const nextDueLabel = useMemo(() => {
    if (!reminder?.enabled) return 'Off'
    if (due) return 'Overdue now'
    if (!reminder.last_snapshot_at) return 'Due now'
    const lastMs = Date.parse(reminder.last_snapshot_at)
    if (Number.isNaN(lastMs)) return 'Scheduled'
    const dueAt = new Date(lastMs + (reminder.interval_days || 7) * 24 * 60 * 60 * 1000)
    return dueAt.toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' })
  }, [reminder, due])

  const needsReplaceConfirm = restoreMode === 'replace'
  const replaceConfirmed =
    !needsReplaceConfirm || replaceConfirmText.trim() === targetServer.name

  return (
    <div className="space-y-5">
      {/* 1. Periodic Backup Reminder */}
      <Panel
        title={
          <>
            <BellRing className="size-4 text-primary" />
            Periodic Snapshot Reminder
          </>
        }
        description="Get a native desktop notification and overview banner when it is time to download a fresh server snapshot."
        actions={
          <Switch
            checked={remEnabled}
            disabled={savingRem}
            onToggle={() => void saveReminder(!remEnabled, remDays)}
            aria-label="Enable periodic snapshot reminder"
          />
        }
      >
        <div className="space-y-4">
          {due && (
            <Banner status="warning" contrast="high" size="compact">
              <BannerTitle>Snapshot backup is due for {server.name}</BannerTitle>
              <BannerDescription>
                {reminder.last_snapshot_at
                  ? `Last downloaded ${relativeTime(reminder.last_snapshot_at)}.`
                  : 'No local snapshot downloaded from Rig Desktop yet.'}
              </BannerDescription>
            </Banner>
          )}

          <div className="flex flex-wrap items-center gap-2">
            {REMINDER_PRESETS.map((p) => {
              const active = Number(remDays) === p.days && remEnabled
              return (
                <button
                  key={p.days}
                  type="button"
                  disabled={savingRem}
                  onClick={() => void saveReminder(true, String(p.days))}
                  className={cn(
                    'inline-flex h-7 items-center rounded-full border px-3 text-[12.5px] transition-colors',
                    active
                      ? 'border-primary bg-primary/12 font-medium text-foreground'
                      : 'border-border bg-surface text-muted-foreground hover:text-foreground',
                  )}
                >
                  {p.label}
                </button>
              )
            })}
            <div className="flex items-center gap-1.5 pl-1">
              <span className="text-[12.5px] text-muted-foreground">Every</span>
              <Input
                size="compact"
                type="number"
                min={1}
                max={365}
                value={remDays}
                onChange={(e) => setRemDays(e.target.value)}
                className="w-18 font-mono"
              />
              <span className="text-[12.5px] text-muted-foreground">days</span>
              <Button
                type="button"
                variant="secondary"
                size="compact"
                loading={savingRem}
                onClick={() => void saveReminder(true, remDays)}
              >
                Save
              </Button>
            </div>
          </div>

          <div className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-border/80 bg-surface/60 px-3.5 py-2.5 text-[12.5px]">
            <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-muted-foreground">
              <span className="inline-flex items-center gap-1.5">
                <Clock className="size-3.5" />
                Last backup:{' '}
                <strong className="font-medium text-foreground">
                  {reminder?.last_snapshot_at
                    ? `${relativeTime(reminder.last_snapshot_at)}${reminder.last_snapshot_bytes ? ` (${formatBytes(reminder.last_snapshot_bytes)})` : ''}`
                    : 'Never'}
                </strong>
              </span>
              <span>
                Next reminder:{' '}
                <strong className={cn('font-medium', due ? 'text-warn' : 'text-foreground')}>
                  {nextDueLabel}
                </strong>
              </span>
            </div>
            {due && (
              <Button
                type="button"
                variant="ghost"
                size="compact"
                disabled={savingRem}
                onClick={() => void handleSnooze()}
              >
                Snooze 24h
              </Button>
            )}
          </div>
        </div>
      </Panel>

      {/* 2. Download Full Server Snapshot */}
      <Panel
        title={
          <>
            <HardDriveDownload className="size-4 text-primary" />
            Download Full Server Snapshot
          </>
        }
        description="Exports a single portable archive containing organization branding, projects, apps, decrypted-for-portability env secrets, domains, volumes, alerts, and live database dumps."
      >
        <form onSubmit={handleExport} className="space-y-4">
          <div className="grid grid-cols-1 gap-2.5 sm:grid-cols-3">
            <label className="flex cursor-pointer items-start gap-2.5 rounded-lg border border-border/80 bg-surface/60 p-3 text-[13px]">
              <Checkbox checked={includeDbs} onCheckedChange={(v) => setIncludeDbs(Boolean(v))} />
              <div>
                <div className="font-medium text-foreground">Database dumps</div>
                <div className="text-[12px] text-muted-foreground">
                  Live dumps of managed databases
                </div>
              </div>
            </label>
            <label className="flex cursor-pointer items-start gap-2.5 rounded-lg border border-border/80 bg-surface/60 p-3 text-[13px]">
              <Checkbox
                checked={includeAlerts}
                onCheckedChange={(v) => setIncludeAlerts(Boolean(v))}
              />
              <div>
                <div className="font-medium text-foreground">Alert channels</div>
                <div className="text-[12px] text-muted-foreground">
                  Webhooks &amp; signing secrets
                </div>
              </div>
            </label>
            <label className="flex cursor-pointer items-start gap-2.5 rounded-lg border border-border/80 bg-surface/60 p-3 text-[13px]">
              <Checkbox
                checked={includeGithub}
                onCheckedChange={(v) => setIncludeGithub(Boolean(v))}
              />
              <div>
                <div className="font-medium text-foreground">GitHub App</div>
                <div className="text-[12px] text-muted-foreground">
                  Private key &amp; installation
                </div>
              </div>
            </label>
          </div>

          <div className="space-y-3 rounded-xl border border-border/80 bg-surface/60 p-3.5">
            <div className="flex items-center justify-between gap-3">
              <div className="flex items-center gap-2">
                <Lock className="size-4 text-primary" />
                <div>
                  <div className="text-[13.5px] font-medium text-foreground">
                    Encrypt archive with passphrase (recommended)
                  </div>
                  <div className="text-[12px] text-muted-foreground">
                    Argon2id key derivation + XChaCha20-Poly1305 authenticated encryption.
                  </div>
                </div>
              </div>
              <Switch
                checked={encrypt}
                onToggle={() => setEncrypt((v) => !v)}
                aria-label="Encrypt snapshot archive"
              />
            </div>

            {encrypt ? (
              <div className="grid grid-cols-1 gap-3 pt-1 sm:grid-cols-2">
                <Field>
                  <FieldLabel htmlFor="snap-exp-pw">Passphrase</FieldLabel>
                  <div className="relative">
                    <Input
                      id="snap-exp-pw"
                      type={showExportPass ? 'text' : 'password'}
                      required
                      minLength={8}
                      value={exportPass}
                      onChange={(e) => setExportPass(e.target.value)}
                      placeholder="At least 8 characters"
                      className="pr-9"
                    />
                    <button
                      type="button"
                      onClick={() => setShowExportPass((v) => !v)}
                      aria-label={showExportPass ? 'Hide passphrase' : 'Show passphrase'}
                      className="absolute top-1/2 right-2.5 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                    >
                      {showExportPass ? <EyeOff className="size-4" /> : <Eye className="size-4" />}
                    </button>
                  </div>
                </Field>
                <Field>
                  <FieldLabel htmlFor="snap-exp-pw2">Confirm Passphrase</FieldLabel>
                  <Input
                    id="snap-exp-pw2"
                    type={showExportPass ? 'text' : 'password'}
                    required
                    minLength={8}
                    value={exportPassConfirm}
                    onChange={(e) => setExportPassConfirm(e.target.value)}
                    placeholder="Repeat passphrase"
                  />
                </Field>
              </div>
            ) : (
              <Banner status="warning" contrast="low" size="compact">
                <BannerDescription>
                  Unencrypted snapshots contain plaintext app secrets and database dumps so they can
                  be restored onto any Rig server. Store the file securely.
                </BannerDescription>
              </Banner>
            )}
          </div>

          <div className="flex flex-wrap items-center justify-between gap-3">
            <span className="text-[12.5px] text-muted-foreground">
              Streams directly to disk — never buffers large archives in browser memory.
            </span>
            <Button
              type="submit"
              variant="primary"
              leadingIcon={Download}
              loading={exporting}
            >
              {exporting ? 'Creating Snapshot…' : 'Download Snapshot…'}
            </Button>
          </div>
        </form>
      </Panel>

      {/* 3. Restore Snapshot to Any Server */}
      <Panel
        title={
          <>
            <Upload className="size-4 text-primary" />
            Restore Snapshot to Any Server
          </>
        }
        description="Migrate or recover onto this server or any other paired Rig server. All secrets are automatically re-encrypted under the target server's vault key, and target users/sessions are preserved."
      >
        <div className="space-y-4">
          <div className="grid grid-cols-1 gap-3.5 sm:grid-cols-2">
            <Field>
              <FieldLabel>Target Rig Server</FieldLabel>
              <Select
                value={targetServerId}
                onValueChange={(val) => {
                  setTargetServerId(val)
                  if (restorePath && (!restorePath.endsWith('.enc') || restorePass.trim())) {
                    void runInspect(restorePath, val, restorePass)
                  }
                }}
              >
                <SelectTrigger className="w-full" placeholder="Choose target server" />
                <SelectContent>
                  {servers.map((s, idx) => (
                    <SelectItem key={s.id} index={idx} value={s.id}>
                      {`${s.name} (${s.url})${s.id === server.id ? ' · current' : ''}`}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <FieldDescription>
                Pick any paired server — great for server migrations or staging clones.
              </FieldDescription>
            </Field>

            <Field>
              <FieldLabel>Snapshot Archive File</FieldLabel>
              <div className="flex items-center gap-2">
                <Input
                  readOnly
                  value={restorePath}
                  placeholder="Choose a .rig-snapshot.tar.zst or .enc file…"
                  className="flex-1 bg-surface/60 font-mono text-[12.5px]"
                />
                <Button
                  type="button"
                  variant="secondary"
                  leadingIcon={FolderOpen}
                  onClick={() => void pickSnapshotFile()}
                >
                  Browse…
                </Button>
              </div>
            </Field>
          </div>

          {restorePath && (
            <div className="flex flex-wrap items-end gap-3">
              <Field className="min-w-[240px] flex-1">
                <FieldLabel htmlFor="snap-res-pw">
                  Archive Passphrase{' '}
                  <span className="font-normal text-muted-foreground">
                    ({restorePath.endsWith('.enc') ? 'required for .enc' : 'if encrypted'})
                  </span>
                </FieldLabel>
                <div className="relative">
                  <Input
                    id="snap-res-pw"
                    type={showRestorePass ? 'text' : 'password'}
                    value={restorePass}
                    onChange={(e) => setRestorePass(e.target.value)}
                    placeholder="Enter snapshot passphrase…"
                    className="pr-9"
                  />
                  <button
                    type="button"
                    onClick={() => setShowRestorePass((v) => !v)}
                    aria-label={showRestorePass ? 'Hide passphrase' : 'Show passphrase'}
                    className="absolute top-1/2 right-2.5 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                  >
                    {showRestorePass ? <EyeOff className="size-4" /> : <Eye className="size-4" />}
                  </button>
                </div>
              </Field>
              <Button
                type="button"
                variant="secondary"
                leadingIcon={FileArchive}
                loading={inspecting}
                onClick={() => void runInspect(restorePath, targetServerId, restorePass)}
              >
                {preview ? 'Re-Inspect Archive' : 'Inspect Snapshot'}
              </Button>
            </div>
          )}

          {inspectError && (
            <Banner role="alert" status="error" contrast="high" size="compact">
              <BannerDescription>{inspectError}</BannerDescription>
            </Banner>
          )}

          {preview && (
            <div className="space-y-4 rounded-xl border border-border/80 bg-surface/60 p-4">
              <div className="flex flex-wrap items-center justify-between gap-2 border-b border-border/70 pb-3">
                <div>
                  <div className="flex flex-wrap items-center gap-2 text-[14.5px] font-semibold text-foreground">
                    <Archive className="size-4 text-primary" />
                    <span>{preview.source_org_name}</span>
                    <Badge variant="solid" color="gray" size="compact" className="font-mono">
                      v{preview.rig_version}
                    </Badge>
                    {preview.encrypted && (
                      <Badge variant="solid" color="emerald" size="compact">
                        <ShieldCheck className="size-3" />
                        Encrypted
                      </Badge>
                    )}
                  </div>
                  <div className="mt-0.5 text-[12px] text-muted-foreground">
                    Taken {relativeTime(preview.created_at)} · Target:{' '}
                    <span className="font-medium text-foreground">{targetServer.name}</span>
                  </div>
                </div>
                {conflictCount > 0 && (
                  <Badge variant="solid" color="amber" size="compact">
                    {conflictCount} existing {conflictCount === 1 ? 'conflict' : 'conflicts'} on{' '}
                    {targetServer.name}
                  </Badge>
                )}
              </div>

              <dl className="grid grid-cols-2 gap-3 text-[13px] sm:grid-cols-5">
                <div className="rounded-lg border border-border/70 bg-card px-3 py-2">
                  <dt className="text-[11.5px] text-muted-foreground">Projects</dt>
                  <dd className="mt-0.5 font-mono text-[16px] font-semibold text-foreground">
                    {preview.projects.length}
                  </dd>
                </div>
                <div className="rounded-lg border border-border/70 bg-card px-3 py-2">
                  <dt className="text-[11.5px] text-muted-foreground">Apps</dt>
                  <dd className="mt-0.5 font-mono text-[16px] font-semibold text-foreground">
                    {preview.apps.length}
                  </dd>
                </div>
                <div className="rounded-lg border border-border/70 bg-card px-3 py-2">
                  <dt className="text-[11.5px] text-muted-foreground">Databases</dt>
                  <dd className="mt-0.5 font-mono text-[16px] font-semibold text-foreground">
                    {preview.databases.length}
                  </dd>
                </div>
                <div className="rounded-lg border border-border/70 bg-card px-3 py-2">
                  <dt className="text-[11.5px] text-muted-foreground">Alerts</dt>
                  <dd className="mt-0.5 font-mono text-[16px] font-semibold text-foreground">
                    {preview.alert_channel_count}
                  </dd>
                </div>
                <div className="rounded-lg border border-border/70 bg-card px-3 py-2">
                  <dt className="text-[11.5px] text-muted-foreground">Media</dt>
                  <dd className="mt-0.5 font-mono text-[16px] font-semibold text-foreground">
                    {preview.media_count}
                  </dd>
                </div>
              </dl>

              {preview.databases.length > 0 && (
                <div className="space-y-1.5">
                  <div className="text-[12px] font-medium text-muted-foreground">
                    Included Databases
                  </div>
                  <div className="flex flex-wrap gap-1.5">
                    {preview.databases.map((db) => (
                      <Badge
                        key={`${db.project_slug}/${db.slug}`}
                        variant="solid"
                        color={db.exists_on_target ? 'amber' : 'gray'}
                        size="compact"
                        className="font-mono"
                      >
                        {db.project_slug}/{db.name} ({db.engine} {db.version}
                        {db.has_dump && db.dump_size_bytes != null
                          ? ` · ${formatBytes(db.dump_size_bytes)}`
                          : ' · schema only'}
                        )
                      </Badge>
                    ))}
                  </div>
                </div>
              )}

              {preview.warnings.length > 0 && (
                <Banner status="warning" contrast="low" size="compact">
                  <BannerTitle>Inspection Notes</BannerTitle>
                  <BannerDescription>
                    <ul className="mt-1 list-disc space-y-0.5 pl-4">
                      {preview.warnings.map((w, i) => (
                        <li key={i}>{w}</li>
                      ))}
                    </ul>
                  </BannerDescription>
                </Banner>
              )}

              {/* Restore Strategy */}
              <div className="grid grid-cols-1 gap-2.5 sm:grid-cols-2">
                <label
                  className={cn(
                    'flex cursor-pointer items-start gap-2.5 rounded-lg border p-3 text-[13px] transition-colors',
                    restoreMode === 'merge'
                      ? 'border-primary bg-primary/8'
                      : 'border-border/80 bg-card hover:bg-hover',
                  )}
                >
                  <input
                    type="radio"
                    name="snap-mode"
                    checked={restoreMode === 'merge'}
                    onChange={() => setRestoreMode('merge')}
                    className="mt-0.5 accent-[var(--primary)]"
                  />
                  <div>
                    <div className="font-medium text-foreground">Merge safely (recommended)</div>
                    <div className="text-[12px] text-muted-foreground">
                      Adds missing projects, apps, and databases. Skips existing name conflicts.
                    </div>
                  </div>
                </label>

                <label
                  className={cn(
                    'flex cursor-pointer items-start gap-2.5 rounded-lg border p-3 text-[13px] transition-colors',
                    restoreMode === 'replace'
                      ? 'border-fail bg-fail/8'
                      : 'border-border/80 bg-card hover:bg-hover',
                  )}
                >
                  <input
                    type="radio"
                    name="snap-mode"
                    checked={restoreMode === 'replace'}
                    onChange={() => setRestoreMode('replace')}
                    className="mt-0.5 accent-[var(--fail)]"
                  />
                  <div>
                    <div className="font-medium text-fail">Replace existing workloads</div>
                    <div className="text-[12px] text-muted-foreground">
                      Clears existing projects, apps, databases, and alerts on {targetServer.name}{' '}
                      before restoring. Target users &amp; devices are kept.
                    </div>
                  </div>
                </label>
              </div>

              <div className="flex flex-wrap gap-x-5 gap-y-2 text-[13px]">
                <label className="inline-flex cursor-pointer items-center gap-2">
                  <Checkbox
                    checked={restoreBranding}
                    onCheckedChange={(v) => setRestoreBranding(Boolean(v))}
                  />
                  <span>Org branding &amp; logo</span>
                </label>
                <label className="inline-flex cursor-pointer items-center gap-2">
                  <Checkbox
                    checked={startDatabases}
                    onCheckedChange={(v) => setStartDatabases(Boolean(v))}
                  />
                  <span>Provision DBs &amp; restore dumps</span>
                </label>
                <label className="inline-flex cursor-pointer items-center gap-2">
                  <Checkbox
                    checked={deployRunningApps}
                    onCheckedChange={(v) => setDeployRunningApps(Boolean(v))}
                  />
                  <span>Auto-deploy running apps</span>
                </label>
                <label className="inline-flex cursor-pointer items-center gap-2">
                  <Checkbox
                    checked={restoreAlerts}
                    onCheckedChange={(v) => setRestoreAlerts(Boolean(v))}
                  />
                  <span>Alert channels</span>
                </label>
                {preview.has_github_app && (
                  <label className="inline-flex cursor-pointer items-center gap-2">
                    <Checkbox
                      checked={restoreGithub}
                      onCheckedChange={(v) => setRestoreGithub(Boolean(v))}
                    />
                    <span>GitHub App</span>
                  </label>
                )}
              </div>

              <div className="flex justify-end border-t border-border/70 pt-3">
                <Button
                  type="button"
                  variant={restoreMode === 'replace' ? 'destructive' : 'primary'}
                  leadingIcon={RotateCcw}
                  onClick={() => {
                    setReplaceConfirmText('')
                    setConfirmOpen(true)
                  }}
                >
                  Restore onto {targetServer.name}…
                </Button>
              </div>
            </div>
          )}

          {report && (
            <Banner status="success" contrast="high">
              <BannerTitle className="flex items-center gap-2">
                <CheckCircle2 className="size-4" />
                Snapshot Restored onto {targetServer.name}
              </BannerTitle>
              <BannerDescription>
                <div className="mt-1 space-y-1">
                  <div>
                    Created {report.projects_created} projects ({report.projects_skipped} skipped),{' '}
                    {report.apps_created} apps ({report.apps_skipped} skipped,{' '}
                    {report.apps_deployments_queued} deploys queued), {report.databases_created}{' '}
                    databases ({report.database_dumps_scheduled} dumps restoring),{' '}
                    {report.env_vars_restored} env vars re-encrypted, {report.domains_restored}{' '}
                    domains, and {report.alerts_created} alerts.
                  </div>
                  {report.warnings.length > 0 && (
                    <ul className="list-disc pl-4 text-[12px]">
                      {report.warnings.map((w, i) => (
                        <li key={i}>{w}</li>
                      ))}
                    </ul>
                  )}
                </div>
              </BannerDescription>
            </Banner>
          )}
        </div>
      </Panel>

      {/* Final Restore Confirmation Dialog */}
      <Dialog open={confirmOpen} onOpenChange={setConfirmOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              {restoreMode === 'replace' ? (
                <AlertTriangle className="size-4.5 text-fail" />
              ) : (
                <Server className="size-4.5 text-primary" />
              )}
              Confirm Snapshot Restore
            </DialogTitle>
            <DialogDescription>
              {restoreMode === 'replace'
                ? `REPLACE mode will remove all existing projects, apps, databases, and alert channels on ${targetServer.name} before restoring from "${preview?.source_org_name}".`
                : `MERGE mode will restore missing projects, apps, databases, and secrets from "${preview?.source_org_name}" onto ${targetServer.name}.`}
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-4">
            {needsReplaceConfirm && (
              <Field>
                <FieldLabel htmlFor="snap-replace-confirm">
                  Type <code className="font-mono text-foreground">{targetServer.name}</code> to
                  confirm replacement
                </FieldLabel>
                <Input
                  id="snap-replace-confirm"
                  type="text"
                  autoFocus
                  value={replaceConfirmText}
                  onChange={(e) => setReplaceConfirmText(e.target.value)}
                  placeholder={targetServer.name}
                  className="font-mono"
                />
              </Field>
            )}

            <DialogFooter>
              <Button type="button" variant="ghost" onClick={() => setConfirmOpen(false)}>
                Cancel
              </Button>
              <Button
                type="button"
                variant={restoreMode === 'replace' ? 'destructive' : 'primary'}
                leadingIcon={RotateCcw}
                loading={restoring}
                disabled={!replaceConfirmed}
                onClick={() => void executeRestore()}
              >
                {restoring ? 'Restoring…' : `Restore to ${targetServer.name}`}
              </Button>
            </DialogFooter>
          </div>
        </DialogContent>
      </Dialog>
    </div>
  )
}
