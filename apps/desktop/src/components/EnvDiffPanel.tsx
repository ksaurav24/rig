import {
  Check,
  Copy,
  Eye,
  EyeOff,
  FileUp,
  Lock,
  Pencil,
  Plus,
  ShieldAlert,
  Trash2,
  Upload,
  X,
} from 'lucide-react'
import { useRef, useState, type DragEvent, type FormEvent } from 'react'
import { toast } from 'sonner'
import { Panel } from '@rig/ui/app/page'
import { Badge } from '@rig/ui/badge'
import { Button } from '@rig/ui/button'
import { Checkbox } from '@rig/ui/checkbox'
import { ConfirmDialog } from '@rig/ui/confirm-dialog'
import { Input } from '@rig/ui/input'
import { Switch } from '@rig/ui/switch'
import { Textarea } from '@rig/ui/textarea'
import { desktopApi } from '@/lib/ipc'
import type { EnvDiffPreview, EnvVar } from '@/lib/types'
import { cn } from '@rig/ui/lib/utils'

interface EnvDiffPanelProps {
  serverId: string
  appId: string
  env: EnvVar[]
  onApplied: () => void
}

export function EnvDiffPanel({ serverId, appId, env, onApplied }: EnvDiffPanelProps) {
  const [newKey, setNewKey] = useState('')
  const [newValue, setNewValue] = useState('')
  const [newSecret, setNewSecret] = useState(false)
  const [savingVar, setSavingVar] = useState(false)

  const [editingKey, setEditingKey] = useState<string | null>(null)
  const [editValue, setEditValue] = useState('')
  const [editSecret, setEditSecret] = useState(false)

  const [revealed, setRevealed] = useState<Record<string, string>>({})
  const [revealingKey, setRevealingKey] = useState<string | null>(null)
  const [deletingKey, setDeletingKey] = useState<string | null>(null)
  const [confirmDeleteKey, setConfirmDeleteKey] = useState<string | null>(null)
  const fileInputRef = useRef<HTMLInputElement>(null)

  const [dragging, setDragging] = useState(false)
  const [preview, setPreview] = useState<EnvDiffPreview | null>(null)
  const [selectedKeys, setSelectedKeys] = useState<Set<string>>(new Set())
  const [busy, setBusy] = useState(false)
  const [pasteMode, setPasteMode] = useState(false)
  const [pasteText, setPasteText] = useState('')

  async function handleAddVar(e: FormEvent) {
    e.preventDefault()
    const key = newKey.trim()
    if (!key) return
    setSavingVar(true)
    try {
      await desktopApi.setEnvVar(serverId, appId, {
        key,
        value: newValue,
        is_secret: newSecret,
      })
      toast.success(`Saved ${key}`)
      setNewKey('')
      setNewValue('')
      setNewSecret(false)
      setRevealed((prev) => {
        const next = { ...prev }
        delete next[key]
        return next
      })
      onApplied()
    } catch (err) {
      toast.error(String(err))
    } finally {
      setSavingVar(false)
    }
  }

  async function startEdit(v: EnvVar) {
    setEditingKey(v.key)
    setEditSecret(v.is_secret)
    if (v.is_secret) {
      if (revealed[v.key] !== undefined) {
        setEditValue(revealed[v.key])
      } else {
        setEditValue('')
        try {
          const res = await desktopApi.revealEnvVar(serverId, appId, v.key)
          setRevealed((prev) => ({ ...prev, [v.key]: res.value }))
          setEditValue(res.value)
        } catch (err) {
          toast.error(`Could not reveal ${v.key}: ${String(err)}`)
        }
      }
    } else {
      setEditValue(v.value ?? '')
    }
  }

  async function saveEdit(key: string) {
    setSavingVar(true)
    try {
      await desktopApi.setEnvVar(serverId, appId, {
        key,
        value: editValue,
        is_secret: editSecret,
      })
      toast.success(`Updated ${key}`)
      setEditingKey(null)
      setRevealed((prev) => {
        const next = { ...prev }
        if (editSecret) next[key] = editValue
        else delete next[key]
        return next
      })
      onApplied()
    } catch (err) {
      toast.error(String(err))
    } finally {
      setSavingVar(false)
    }
  }

  async function toggleReveal(key: string) {
    if (revealed[key] !== undefined) {
      setRevealed((prev) => {
        const next = { ...prev }
        delete next[key]
        return next
      })
      return
    }
    setRevealingKey(key)
    try {
      const res = await desktopApi.revealEnvVar(serverId, appId, key)
      setRevealed((prev) => ({ ...prev, [key]: res.value }))
    } catch (err) {
      toast.error(String(err))
    } finally {
      setRevealingKey(null)
    }
  }

  async function handleDelete(key: string) {
    setDeletingKey(key)
    try {
      await desktopApi.deleteEnvVar(serverId, appId, key)
      toast.success(`Deleted ${key}`)
      setRevealed((prev) => {
        const next = { ...prev }
        delete next[key]
        return next
      })
      if (editingKey === key) setEditingKey(null)
      onApplied()
    } catch (err) {
      toast.error(String(err))
    } finally {
      setDeletingKey(null)
    }
  }

  async function runDiff(content: string) {
    setBusy(true)
    try {
      const diff = await desktopApi.previewEnvDiff(serverId, appId, content)
      setPreview(diff)
      setSelectedKeys(
        new Set(diff.items.filter((i) => i.kind !== 'unchanged').map((i) => i.key)),
      )
      setPasteMode(false)
    } catch (e) {
      toast.error(String(e))
    } finally {
      setBusy(false)
    }
  }

  function handleDrop(e: DragEvent<HTMLDivElement>) {
    e.preventDefault()
    setDragging(false)
    const file = e.dataTransfer.files[0]
    if (!file) return
    void file.text().then(runDiff)
  }

  async function applyDiff() {
    if (!preview) return
    const chosen = preview.items.filter((i) => selectedKeys.has(i.key) && i.kind !== 'unchanged')
    if (chosen.length === 0) {
      toast.info('No changes selected to apply')
      return
    }
    setBusy(true)
    try {
      await desktopApi.applyEnvDiff(serverId, appId, chosen)
      toast.success(
        `Applied ${chosen.length} environment variable${chosen.length === 1 ? '' : 's'}`,
      )
      setPreview(null)
      setRevealed({})
      onApplied()
    } catch (e) {
      toast.error(String(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="space-y-5">
      {/* Add Single Env Variable */}
      <Panel>
        <form onSubmit={handleAddVar} className="flex flex-wrap items-center gap-2.5">
          <Input
            type="text"
            required
            value={newKey}
            onChange={(e) => setNewKey(e.target.value.toUpperCase().replace(/[^A-Z0-9_]/g, ''))}
            placeholder="KEY_NAME"
            className="w-52 font-mono"
          />
          <Input
            type={newSecret ? 'password' : 'text'}
            value={newValue}
            onChange={(e) => setNewValue(e.target.value)}
            placeholder="value"
            className="min-w-[200px] flex-1 font-mono"
          />
          <Switch
            label="Secret"
            size="compact"
            checked={newSecret}
            onToggle={() => setNewSecret((v) => !v)}
          />
          <Button
            type="submit"
            variant="primary"
            leadingIcon={Plus}
            loading={savingVar}
            disabled={!newKey.trim()}
          >
            Add Variable
          </Button>
        </form>
      </Panel>

      {/* Current Env Vars */}
      <Panel
        title={`Environment Variables (${env.length})`}
        description="Changes apply on next deploy or restart."
        flush
      >
        {env.length === 0 ? (
          <div className="px-5 py-8 text-center text-[13.5px] text-muted-foreground">
            No environment variables configured yet. Add one above or drop a{' '}
            <code className="rounded bg-surface px-1.5 py-0.5 font-mono text-foreground">.env</code>{' '}
            file below.
          </div>
        ) : (
          <div className="divide-y divide-border/80 font-mono text-[13px]">
            {env.map((v) => {
              const isEditing = editingKey === v.key
              const revealedVal = revealed[v.key]
              const displayValue = v.is_secret
                ? revealedVal !== undefined
                  ? revealedVal
                  : null
                : (v.value ?? '')

              if (isEditing) {
                return (
                  <div
                    key={v.key}
                    className="flex flex-wrap items-center gap-2.5 bg-hover px-5 py-3"
                  >
                    <span className="w-52 shrink-0 truncate font-semibold text-foreground">
                      {v.key}
                    </span>
                    <Input
                      size="compact"
                      type="text"
                      value={editValue}
                      onChange={(e) => setEditValue(e.target.value)}
                      className="min-w-[200px] flex-1 font-mono"
                      autoFocus
                    />
                    <Switch
                      label="Secret"
                      size="compact"
                      checked={editSecret}
                      onToggle={() => setEditSecret((s) => !s)}
                    />
                    <Button
                      type="button"
                      variant="primary"
                      size="compact"
                      leadingIcon={Check}
                      loading={savingVar}
                      onClick={() => void saveEdit(v.key)}
                    >
                      Save
                    </Button>
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon-compact"
                      onClick={() => setEditingKey(null)}
                      aria-label="Cancel edit"
                    >
                      <X />
                    </Button>
                  </div>
                )
              }

              return (
                <div
                  key={v.key}
                  className="flex items-center gap-3 px-5 py-2.5 transition-colors hover:bg-hover"
                >
                  <span className="w-56 shrink-0 truncate font-semibold text-foreground">
                    {v.key}
                  </span>
                  <span className="min-w-0 flex-1 truncate text-muted-foreground">
                    {displayValue === null ? (
                      <span className="inline-flex items-center gap-1.5 text-warn">
                        <Lock className="size-3.5" />
                        •••••••• (secret)
                      </span>
                    ) : (
                      <span className={v.is_secret ? 'text-warn' : 'text-foreground/90'}>
                        {displayValue}
                      </span>
                    )}
                  </span>
                  <div className="flex shrink-0 items-center gap-1">
                    {v.is_secret && (
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon-compact"
                        disabled={revealingKey === v.key}
                        onClick={() => void toggleReveal(v.key)}
                        title={revealedVal !== undefined ? 'Hide secret' : 'Reveal secret'}
                        aria-label={revealedVal !== undefined ? 'Hide secret' : 'Reveal secret'}
                      >
                        {revealedVal !== undefined ? <EyeOff /> : <Eye />}
                      </Button>
                    )}
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon-compact"
                      onClick={async () => {
                        let val = displayValue
                        if (val === null) {
                          try {
                            const res = await desktopApi.revealEnvVar(serverId, appId, v.key)
                            val = res.value
                          } catch (err) {
                            toast.error(String(err))
                            return
                          }
                        }
                        void navigator.clipboard.writeText(val)
                        toast.success(`Copied ${v.key}`)
                      }}
                      title="Copy value"
                      aria-label={`Copy ${v.key}`}
                    >
                      <Copy />
                    </Button>
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon-compact"
                      onClick={() => void startEdit(v)}
                      title="Edit variable"
                      aria-label={`Edit ${v.key}`}
                    >
                      <Pencil />
                    </Button>
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon-compact"
                      disabled={deletingKey === v.key}
                      onClick={() => setConfirmDeleteKey(v.key)}
                      title="Delete variable"
                      aria-label={`Delete ${v.key}`}
                      className="text-muted-foreground hover:text-fail"
                    >
                      <Trash2 />
                    </Button>
                  </div>
                </div>
              )
            })}
          </div>
        )}
      </Panel>

      {/* Drag-and-drop .env bulk importer */}
      <div
        onDragOver={(e) => {
          e.preventDefault()
          setDragging(true)
        }}
        onDragLeave={() => setDragging(false)}
        onDrop={handleDrop}
        className={cn(
          'flex flex-col items-center justify-center rounded-xl border border-dashed px-6 py-6 text-center transition-colors',
          dragging
            ? 'border-primary bg-primary/10'
            : 'border-border bg-card/60 hover:border-primary/40',
        )}
      >
        <FileUp className="mb-2 size-5 text-primary" />
        <div className="text-[14px] font-medium text-foreground">
          Bulk import: drop a{' '}
          <code className="rounded bg-surface px-1.5 py-0.5 font-mono">.env</code> file to diff
          locally in Rust
        </div>
        <p className="mt-1 text-[12.5px] text-muted-foreground">
          Parsed on-device · existing secrets stay masked · applied in one atomic batch request
        </p>
        <div className="mt-4 flex flex-wrap items-center gap-2">
          <input
            ref={fileInputRef}
            type="file"
            accept=".env,.env.*,text/plain"
            className="hidden"
            onChange={(e) => {
              const f = e.target.files?.[0]
              if (f) void f.text().then(runDiff)
              e.target.value = ''
            }}
          />
          <Button
            type="button"
            variant="tertiary"
            size="compact"
            leadingIcon={Upload}
            onClick={() => fileInputRef.current?.click()}
          >
            Choose .env file
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="compact"
            onClick={() => setPasteMode((v) => !v)}
          >
            {pasteMode ? 'Close paste box' : 'Or paste .env'}
          </Button>
        </div>
      </div>

      {pasteMode && (
        <Panel>
          <div className="space-y-3">
            <Textarea
              rows={5}
              value={pasteText}
              onChange={(e) => setPasteText(e.target.value)}
              placeholder={'DATABASE_URL=postgres://...\nNODE_ENV=production'}
              className="font-mono"
            />
            <div className="flex justify-end">
              <Button
                type="button"
                variant="primary"
                loading={busy}
                disabled={!pasteText.trim()}
                onClick={() => void runDiff(pasteText)}
              >
                Preview Diff
              </Button>
            </div>
          </div>
        </Panel>
      )}

      {/* Diff preview */}
      {preview && (
        <Panel
          title=".env Diff Preview"
          description={
            <span className="mt-1 inline-flex flex-wrap items-center gap-2">
              <Badge variant="solid" color="emerald" size="compact" className="font-mono">
                +{preview.added} added
              </Badge>
              <Badge variant="solid" color="blue" size="compact" className="font-mono">
                ~{preview.changed} changed
              </Badge>
              {preview.secret_overwrites > 0 && (
                <Badge
                  variant="solid"
                  color="amber"
                  size="compact"
                  icon={ShieldAlert}
                  className="font-mono"
                >
                  {preview.secret_overwrites} secret overwrite
                </Badge>
              )}
              <span className="font-mono text-[12px] text-muted-foreground">
                {preview.unchanged} unchanged
              </span>
            </span>
          }
          actions={
            <>
              <Button
                type="button"
                variant="ghost"
                size="compact"
                onClick={() => setPreview(null)}
              >
                Discard
              </Button>
              <Button
                type="button"
                variant="primary"
                size="compact"
                leadingIcon={Check}
                loading={busy}
                disabled={selectedKeys.size === 0}
                onClick={applyDiff}
              >
                Apply {selectedKeys.size} changes
              </Button>
            </>
          }
          flush
        >
          <div className="divide-y divide-border/80 font-mono text-[13px]">
            {preview.items.map((item) => {
              const selectable = item.kind !== 'unchanged'
              const checked = selectedKeys.has(item.key)
              return (
                <label
                  key={item.key}
                  className={cn(
                    'flex items-center gap-3 px-5 py-2.5',
                    selectable ? 'cursor-pointer hover:bg-hover' : 'opacity-55',
                  )}
                >
                  <Checkbox
                    disabled={!selectable}
                    checked={checked}
                    onCheckedChange={(isChecked) => {
                      const next = new Set(selectedKeys)
                      if (isChecked) next.add(item.key)
                      else next.delete(item.key)
                      setSelectedKeys(next)
                    }}
                  />
                  <Badge
                    variant="solid"
                    size="compact"
                    color={
                      item.kind === 'added'
                        ? 'emerald'
                        : item.kind === 'changed'
                          ? 'blue'
                          : item.kind === 'secret_overwrite'
                            ? 'amber'
                            : 'gray'
                    }
                    className="w-24 justify-center uppercase"
                  >
                    {item.kind === 'secret_overwrite' ? 'secret' : item.kind}
                  </Badge>
                  <span className="w-52 shrink-0 truncate font-semibold text-foreground">
                    {item.key}
                  </span>
                  <span className="min-w-0 flex-1 truncate text-muted-foreground">
                    {item.kind === 'changed' && (
                      <>
                        <span className="text-fail line-through">{item.current_value}</span>
                        <span className="mx-2 text-muted-foreground">→</span>
                      </>
                    )}
                    <span className="text-foreground/90">{item.incoming_preview}</span>
                  </span>
                </label>
              )
            })}
          </div>
        </Panel>
      )}

      <ConfirmDialog
        open={Boolean(confirmDeleteKey)}
        onOpenChange={(open) => {
          if (!open) setConfirmDeleteKey(null)
        }}
        title="Delete Environment Variable"
        description="Removes this variable from the application configuration. Changes take effect on the next deploy or restart."
        target={confirmDeleteKey ?? undefined}
        confirmLabel="Delete Variable"
        tone="danger"
        onConfirm={async () => {
          if (confirmDeleteKey) await handleDelete(confirmDeleteKey)
        }}
      />
    </div>
  )
}
