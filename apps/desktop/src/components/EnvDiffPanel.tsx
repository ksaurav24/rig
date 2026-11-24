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
import { useState, type DragEvent, type FormEvent } from 'react'
import { toast } from 'sonner'
import { desktopApi } from '@/lib/ipc'
import type { EnvDiffPreview, EnvVar } from '@/lib/types'

interface EnvDiffPanelProps {
  serverId: string
  appId: string
  env: EnvVar[]
  onApplied: () => void
}

export function EnvDiffPanel({ serverId, appId, env, onApplied }: EnvDiffPanelProps) {
  // Single variable CRUD state
  const [newKey, setNewKey] = useState('')
  const [newValue, setNewValue] = useState('')
  const [newSecret, setNewSecret] = useState(false)
  const [savingVar, setSavingVar] = useState(false)

  // Inline edit state
  const [editingKey, setEditingKey] = useState<string | null>(null)
  const [editValue, setEditValue] = useState('')
  const [editSecret, setEditSecret] = useState(false)

  // Revealed secrets cache (key -> plaintext value)
  const [revealed, setRevealed] = useState<Record<string, string>>({})
  const [revealingKey, setRevealingKey] = useState<string | null>(null)
  const [deletingKey, setDeletingKey] = useState<string | null>(null)

  // Bulk .env drag-and-drop / paste diff state
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
      toast.success(`Applied ${chosen.length} environment variable${chosen.length === 1 ? '' : 's'}`)
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
    <div className="space-y-4">
      {/* Add Single Env Variable */}
      <form
        onSubmit={handleAddVar}
        className="flex flex-wrap items-center gap-2 rounded-xl border border-line bg-surface p-3.5 text-xs"
      >
        <input
          type="text"
          required
          value={newKey}
          onChange={(e) => setNewKey(e.target.value.toUpperCase().replace(/[^A-Z0-9_]/g, ''))}
          placeholder="KEY_NAME"
          className="w-44 rounded-lg border border-line bg-bg px-3 py-1.5 font-mono text-xs text-fg outline-none focus:border-emerald-500"
        />
        <input
          type={newSecret ? 'password' : 'text'}
          value={newValue}
          onChange={(e) => setNewValue(e.target.value)}
          placeholder="value"
          className="min-w-[180px] flex-1 rounded-lg border border-line bg-bg px-3 py-1.5 font-mono text-xs text-fg outline-none focus:border-emerald-500"
        />
        <label className="inline-flex cursor-pointer items-center gap-1.5 rounded-lg border border-line bg-bg px-2.5 py-1.5 text-xs text-muted hover:text-fg">
          <input
            type="checkbox"
            checked={newSecret}
            onChange={(e) => setNewSecret(e.target.checked)}
            className="accent-emerald-500"
          />
          <Lock className="size-3 text-amber-400" />
          Secret
        </label>
        <button
          type="submit"
          disabled={savingVar || !newKey.trim()}
          className="inline-flex items-center gap-1.5 rounded-lg bg-emerald-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-emerald-500 disabled:opacity-50"
        >
          <Plus className="size-3.5" />
          Add Variable
        </button>
      </form>

      {/* Current Env Vars with Reveal / Edit / Copy / Delete */}
      <div className="overflow-hidden rounded-xl border border-line bg-surface">
        <div className="flex items-center justify-between border-b border-line bg-surface-2/50 px-4 py-2 text-xs font-medium text-muted">
          <span>Environment Variables ({env.length})</span>
          <span className="text-[11px] font-normal">
            Changes apply on next deploy or restart
          </span>
        </div>
        {env.length === 0 ? (
          <div className="px-4 py-6 text-center text-xs text-muted">
            No environment variables configured yet. Add one above or drop a{' '}
            <code className="rounded bg-surface-2 px-1 py-0.5 font-mono">.env</code> file below.
          </div>
        ) : (
          <div className="divide-y divide-line font-mono text-xs">
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
                  <div key={v.key} className="flex flex-wrap items-center gap-2 bg-surface-2/40 px-4 py-2.5">
                    <span className="w-48 shrink-0 truncate font-semibold text-fg">{v.key}</span>
                    <input
                      type="text"
                      value={editValue}
                      onChange={(e) => setEditValue(e.target.value)}
                      className="min-w-[180px] flex-1 rounded-md border border-emerald-500/60 bg-bg px-2.5 py-1 text-xs text-fg outline-none"
                      autoFocus
                    />
                    <label className="inline-flex cursor-pointer items-center gap-1 font-sans text-[11px] text-muted">
                      <input
                        type="checkbox"
                        checked={editSecret}
                        onChange={(e) => setEditSecret(e.target.checked)}
                        className="accent-emerald-500"
                      />
                      Secret
                    </label>
                    <button
                      type="button"
                      disabled={savingVar}
                      onClick={() => void saveEdit(v.key)}
                      className="inline-flex items-center gap-1 rounded-md bg-emerald-600 px-2.5 py-1 font-sans text-[11px] font-medium text-white hover:bg-emerald-500 disabled:opacity-50"
                    >
                      <Check className="size-3" />
                      Save
                    </button>
                    <button
                      type="button"
                      onClick={() => setEditingKey(null)}
                      className="rounded-md p-1 text-muted hover:bg-bg hover:text-fg"
                    >
                      <X className="size-3.5" />
                    </button>
                  </div>
                )
              }

              return (
                <div key={v.key} className="flex items-center gap-3 px-4 py-2 hover:bg-surface-2/30">
                  <span className="w-52 shrink-0 truncate font-semibold text-fg">{v.key}</span>
                  <span className="min-w-0 flex-1 truncate text-muted">
                    {displayValue === null ? (
                      <span className="inline-flex items-center gap-1 text-amber-300/80">
                        <Lock className="size-3" />
                        •••••••• (secret)
                      </span>
                    ) : (
                      <span className={v.is_secret ? 'text-amber-200' : 'text-fg/90'}>
                        {displayValue}
                      </span>
                    )}
                  </span>
                  <div className="flex shrink-0 items-center gap-1">
                    {v.is_secret && (
                      <button
                        type="button"
                        disabled={revealingKey === v.key}
                        onClick={() => void toggleReveal(v.key)}
                        title={revealedVal !== undefined ? 'Hide secret' : 'Reveal secret'}
                        className="rounded-md p-1 text-muted hover:bg-surface-2 hover:text-fg"
                      >
                        {revealedVal !== undefined ? (
                          <EyeOff className="size-3.5" />
                        ) : (
                          <Eye className="size-3.5" />
                        )}
                      </button>
                    )}
                    <button
                      type="button"
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
                      className="rounded-md p-1 text-muted hover:bg-surface-2 hover:text-fg"
                    >
                      <Copy className="size-3.5" />
                    </button>
                    <button
                      type="button"
                      onClick={() => void startEdit(v)}
                      title="Edit variable"
                      className="rounded-md p-1 text-muted hover:bg-surface-2 hover:text-fg"
                    >
                      <Pencil className="size-3.5" />
                    </button>
                    <button
                      type="button"
                      disabled={deletingKey === v.key}
                      onClick={() => void handleDelete(v.key)}
                      title="Delete variable"
                      className="rounded-md p-1 text-muted hover:bg-rose-500/15 hover:text-rose-300"
                    >
                      <Trash2 className="size-3.5" />
                    </button>
                  </div>
                </div>
              )
            })}
          </div>
        )}
      </div>

      {/* Drag-and-drop .env bulk importer (§9.2) */}
      <div
        onDragOver={(e) => {
          e.preventDefault()
          setDragging(true)
        }}
        onDragLeave={() => setDragging(false)}
        onDrop={handleDrop}
        className={`flex flex-col items-center justify-center rounded-xl border border-dashed px-6 py-5 text-center transition-colors ${
          dragging
            ? 'border-emerald-400 bg-emerald-500/10'
            : 'border-line bg-surface hover:border-emerald-500/40'
        }`}
      >
        <FileUp className="mb-1.5 size-5 text-emerald-400" />
        <div className="text-xs font-medium text-fg">
          Bulk import: drop a <code className="rounded bg-surface-2 px-1 py-0.5 font-mono">.env</code>{' '}
          file to diff locally in Rust
        </div>
        <p className="mt-1 text-[11px] text-muted">
          Parsed on-device · existing secrets stay masked · applied in one atomic batch request
        </p>
        <div className="mt-3 flex items-center gap-2">
          <label className="inline-flex cursor-pointer items-center gap-1.5 rounded-lg border border-line bg-surface-2 px-3 py-1.5 text-xs font-medium text-fg hover:bg-bg">
            <Upload className="size-3.5" />
            Choose .env file
            <input
              type="file"
              accept=".env,.env.*,text/plain"
              className="hidden"
              onChange={(e) => {
                const f = e.target.files?.[0]
                if (f) void f.text().then(runDiff)
                e.target.value = ''
              }}
            />
          </label>
          <button
            type="button"
            onClick={() => setPasteMode((v) => !v)}
            className="rounded-lg px-3 py-1.5 text-xs text-muted hover:bg-surface-2 hover:text-fg"
          >
            {pasteMode ? 'Close paste box' : 'Or paste .env'}
          </button>
        </div>
      </div>

      {pasteMode && (
        <div className="space-y-2 rounded-xl border border-line bg-surface p-3.5">
          <textarea
            rows={5}
            value={pasteText}
            onChange={(e) => setPasteText(e.target.value)}
            placeholder={'DATABASE_URL=postgres://...\nNODE_ENV=production'}
            className="w-full rounded-lg border border-line bg-bg p-2.5 font-mono text-xs text-fg outline-none focus:border-emerald-500"
          />
          <div className="flex justify-end">
            <button
              type="button"
              disabled={busy || !pasteText.trim()}
              onClick={() => void runDiff(pasteText)}
              className="rounded-lg bg-emerald-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-emerald-500 disabled:opacity-50"
            >
              Preview Diff
            </button>
          </div>
        </div>
      )}

      {/* Diff preview */}
      {preview && (
        <div className="overflow-hidden rounded-xl border border-emerald-500/30 bg-surface">
          <div className="flex flex-wrap items-center justify-between gap-2 border-b border-line bg-surface-2/70 px-4 py-2.5 text-xs">
            <div className="flex items-center gap-3">
              <span className="font-semibold text-fg">.env Diff Preview</span>
              <span className="rounded bg-emerald-500/15 px-1.5 py-0.5 font-mono text-[11px] text-emerald-300">
                +{preview.added} added
              </span>
              <span className="rounded bg-sky-500/15 px-1.5 py-0.5 font-mono text-[11px] text-sky-300">
                ~{preview.changed} changed
              </span>
              {preview.secret_overwrites > 0 && (
                <span className="inline-flex items-center gap-1 rounded bg-amber-500/15 px-1.5 py-0.5 font-mono text-[11px] text-amber-300">
                  <ShieldAlert className="size-3" />
                  {preview.secret_overwrites} secret overwrite
                </span>
              )}
              <span className="font-mono text-[11px] text-muted">
                {preview.unchanged} unchanged
              </span>
            </div>
            <div className="flex items-center gap-2">
              <button
                type="button"
                onClick={() => setPreview(null)}
                className="rounded-md px-2.5 py-1 text-xs text-muted hover:bg-bg hover:text-fg"
              >
                Discard
              </button>
              <button
                type="button"
                disabled={busy || selectedKeys.size === 0}
                onClick={applyDiff}
                className="inline-flex items-center gap-1.5 rounded-md bg-emerald-600 px-3 py-1 text-xs font-medium text-white hover:bg-emerald-500 disabled:opacity-50"
              >
                <Check className="size-3.5" />
                {busy ? 'Applying…' : `Apply ${selectedKeys.size} changes`}
              </button>
            </div>
          </div>

          <div className="divide-y divide-line font-mono text-xs">
            {preview.items.map((item) => {
              const selectable = item.kind !== 'unchanged'
              const checked = selectedKeys.has(item.key)
              return (
                <label
                  key={item.key}
                  className={`flex items-center gap-3 px-4 py-2 ${
                    selectable ? 'cursor-pointer hover:bg-surface-2/50' : 'opacity-55'
                  }`}
                >
                  <input
                    type="checkbox"
                    disabled={!selectable}
                    checked={checked}
                    onChange={(e) => {
                      const next = new Set(selectedKeys)
                      if (e.target.checked) next.add(item.key)
                      else next.delete(item.key)
                      setSelectedKeys(next)
                    }}
                    className="accent-emerald-500"
                  />
                  <span
                    className={`w-28 shrink-0 rounded px-1.5 py-0.5 text-center text-[10px] font-medium uppercase ${
                      item.kind === 'added'
                        ? 'bg-emerald-500/15 text-emerald-300'
                        : item.kind === 'changed'
                          ? 'bg-sky-500/15 text-sky-300'
                          : item.kind === 'secret_overwrite'
                            ? 'bg-amber-500/15 text-amber-300'
                            : 'bg-surface-2 text-muted'
                    }`}
                  >
                    {item.kind === 'secret_overwrite' ? 'secret' : item.kind}
                  </span>
                  <span className="w-48 shrink-0 truncate font-semibold text-fg">{item.key}</span>
                  <span className="min-w-0 flex-1 truncate text-muted">
                    {item.kind === 'changed' && (
                      <>
                        <span className="text-rose-300/80 line-through">{item.current_value}</span>
                        <span className="mx-2 text-muted">→</span>
                      </>
                    )}
                    <span className="text-fg/90">{item.incoming_preview}</span>
                  </span>
                </label>
              )
            })}
          </div>
        </div>
      )}
    </div>
  )
}
