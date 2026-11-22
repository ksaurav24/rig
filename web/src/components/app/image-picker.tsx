import { Link2, Sparkles, Trash2, Upload } from 'lucide-react'
import { useRef, useState, type ChangeEvent, type DragEvent } from 'react'
import { AVATAR_PRESETS } from '@/components/app/user-avatar'
import { Field, FieldDescription, FieldError, FieldLabel } from '@/components/fluid/field'
import { Input } from '@/components/fluid/input'
import { Tabs, TabsItem } from '@/components/fluid/tabs'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import { cn } from '@/lib/utils'

const MAX_RAW_UPLOAD_BYTES = 5 * 1024 * 1024 // 5 MB raw before client resize
const MAX_SERVER_BYTES = 750 * 1024
const ACCEPTED_TYPES = 'image/png,image/jpeg,image/webp,image/gif,image/svg+xml'

/** Resizes large raster images client-side to a crisp square/bounded PNG/WebP. */
async function prepareImageUpload(
  file: File,
  { square = false, maxDim = 256 }: { square?: boolean; maxDim?: number },
): Promise<{ content_type: string; data_base64: string }> {
  if (file.size > MAX_RAW_UPLOAD_BYTES) {
    throw new Error('Choose an image smaller than 5 MB.')
  }
  const type = file.type.toLowerCase()
  const allowed = new Set(['image/png', 'image/jpeg', 'image/webp', 'image/gif', 'image/svg+xml'])
  if (!allowed.has(type)) {
    throw new Error('Use a PNG, JPEG, WebP, GIF, or SVG file.')
  }

  // Keep SVG and small GIFs intact (preserves vector sharpness / animation).
  if (type === 'image/svg+xml' || (type === 'image/gif' && file.size <= MAX_SERVER_BYTES)) {
    const buf = await file.arrayBuffer()
    if (buf.byteLength > MAX_SERVER_BYTES) {
      throw new Error('SVG and GIF uploads must be 750 KB or smaller.')
    }
    return { content_type: type, data_base64: bufferToBase64(buf) }
  }

  const bitmap = await loadImageBitmap(file)
  const { width, height } = bitmap
  let sx = 0
  let sy = 0
  let sw = width
  let sh = height
  let dw = width
  let dh = height

  if (square) {
    const side = Math.min(width, height)
    sx = Math.floor((width - side) / 2)
    sy = Math.floor((height - side) / 2)
    sw = side
    sh = side
    dw = Math.min(side, maxDim)
    dh = dw
  } else if (width > maxDim || height > maxDim) {
    const scale = maxDim / Math.max(width, height)
    dw = Math.max(1, Math.round(width * scale))
    dh = Math.max(1, Math.round(height * scale))
  }

  const canvas = document.createElement('canvas')
  canvas.width = dw
  canvas.height = dh
  const ctx = canvas.getContext('2d')
  if (!ctx) throw new Error('Browser canvas is unavailable.')
  ctx.drawImage(bitmap, sx, sy, sw, sh, 0, 0, dw, dh)
  if ('close' in bitmap && typeof bitmap.close === 'function') bitmap.close()

  const outType = type === 'image/jpeg' ? 'image/jpeg' : 'image/png'
  const blob = await new Promise<Blob | null>((resolve) =>
    canvas.toBlob(resolve, outType, outType === 'image/jpeg' ? 0.88 : undefined),
  )
  if (!blob) throw new Error('Could not process that image.')
  const buf = await blob.arrayBuffer()
  if (buf.byteLength > MAX_SERVER_BYTES) {
    throw new Error('Processed image is still larger than 750 KB.')
  }
  return { content_type: outType, data_base64: bufferToBase64(buf) }
}

function bufferToBase64(buffer: ArrayBuffer): string {
  const bytes = new Uint8Array(buffer)
  let binary = ''
  const chunk = 0x8000
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunk))
  }
  return btoa(binary)
}

async function loadImageBitmap(file: File): Promise<HTMLImageElement | ImageBitmap> {
  if (typeof createImageBitmap === 'function') {
    try {
      return await createImageBitmap(file)
    } catch {
      // Fall back to HTMLImageElement below.
    }
  }
  const url = URL.createObjectURL(file)
  try {
    return await new Promise<HTMLImageElement>((resolve, reject) => {
      const img = new Image()
      img.onload = () => resolve(img)
      img.onerror = () => reject(new Error('Could not decode that image file.'))
      img.src = url
    })
  } finally {
    URL.revokeObjectURL(url)
  }
}

export interface ImagePickerProps {
  /** Current saved URL or preset ID. */
  currentUrl?: string | null
  /** Show the built-in avatar presets tab. */
  showPresets?: boolean
  /** Crop uploaded raster images to a square (avatars). */
  square?: boolean
  /** Pending state from parent mutation. */
  pending?: boolean
  /** Called with base64 payload when the user uploads a file. */
  onUpload: (payload: { content_type: string; data_base64: string }) => void
  /** Called when the user saves an external http(s) image URL. */
  onSetUrl: (url: string) => void
  /** Called when the user clicks a built-in preset. */
  onSelectPreset?: (presetId: string) => void
  /** Called when the user clicks Remove / Reset. */
  onRemove?: () => void
  /** Label for the remove button. */
  removeLabel?: string
}

export function ImagePicker({
  currentUrl,
  showPresets = false,
  square = false,
  pending = false,
  onUpload,
  onSetUrl,
  onSelectPreset,
  onRemove,
  removeLabel = 'Reset to default',
}: ImagePickerProps) {
  const modes = showPresets
    ? ([
        { id: 'presets', label: 'Presets', icon: Sparkles },
        { id: 'upload', label: 'Upload', icon: Upload },
        { id: 'link', label: 'Image link', icon: Link2 },
      ] as const)
    : ([
        { id: 'upload', label: 'Upload', icon: Upload },
        { id: 'link', label: 'Image link', icon: Link2 },
      ] as const)

  const initialExternal =
    currentUrl && (currentUrl.startsWith('http://') || currentUrl.startsWith('https://')) ? currentUrl : ''

  const [tabIndex, setTabIndex] = useState(0)
  const [urlValue, setUrlValue] = useState(initialExternal)
  const [urlBroken, setUrlBroken] = useState(false)
  const [dragging, setDragging] = useState(false)
  const [localError, setLocalError] = useState('')
  const [processing, setProcessing] = useState(false)
  const inputRef = useRef<HTMLInputElement>(null)

  const activeMode = modes[tabIndex]?.id ?? 'upload'
  const busy = pending || processing

  async function handleFile(file: File | undefined) {
    if (!file) return
    setLocalError('')
    setProcessing(true)
    try {
      const payload = await prepareImageUpload(file, { square })
      onUpload(payload)
    } catch (e) {
      setLocalError(e instanceof Error ? e.message : 'Could not read that image.')
    } finally {
      setProcessing(false)
      if (inputRef.current) inputRef.current.value = ''
    }
  }

  function onFileChange(e: ChangeEvent<HTMLInputElement>) {
    void handleFile(e.target.files?.[0])
  }

  function onDrop(e: DragEvent<HTMLDivElement>) {
    e.preventDefault()
    setDragging(false)
    void handleFile(e.dataTransfer.files?.[0])
  }

  function applyLink() {
    const trimmed = urlValue.trim()
    setLocalError('')
    if (!/^https?:\/\/.+/i.test(trimmed)) {
      setLocalError('Enter a full http:// or https:// image URL.')
      return
    }
    onSetUrl(trimmed)
  }

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <Tabs
          size="compact"
          selectedIndex={tabIndex}
          onSelect={(i) => {
            setLocalError('')
            setTabIndex(i)
          }}
          aria-label="Image source"
          className="border border-border/80 bg-surface/60"
        >
          {modes.map((m, i) => (
            <TabsItem key={m.id} index={i} icon={m.icon} label={m.label} />
          ))}
        </Tabs>

        {currentUrl && onRemove ? (
          <Button type="button" variant="ghost" size="sm" disabled={busy} onClick={onRemove}>
            <Trash2 data-icon="inline-start" />
            {removeLabel}
          </Button>
        ) : null}
      </div>

      {activeMode === 'presets' && showPresets ? (
        <div className="grid grid-cols-6 gap-2 pt-0.5 sm:grid-cols-6">
          {AVATAR_PRESETS.map((preset) => {
            const selected = currentUrl === preset.id
            return (
              <button
                key={preset.id}
                type="button"
                disabled={busy}
                title={preset.label}
                aria-label={`Choose ${preset.label} avatar`}
                aria-pressed={selected}
                onClick={() => onSelectPreset?.(preset.id)}
                className={cn(
                  'group relative aspect-square overflow-hidden rounded-xl border transition-all',
                  selected
                    ? 'border-primary ring-2 ring-primary/35'
                    : 'border-border/80 hover:scale-[1.04] hover:border-foreground/30',
                )}
              >
                {preset.render('size-full block')}
              </button>
            )
          })}
        </div>
      ) : null}

      {activeMode === 'upload' ? (
        <div
          onDragOver={(e) => {
            e.preventDefault()
            setDragging(true)
          }}
          onDragLeave={() => setDragging(false)}
          onDrop={onDrop}
          className={cn(
            'flex flex-col items-center justify-center gap-2 rounded-xl border border-dashed px-4 py-5 text-center transition-colors',
            dragging
              ? 'border-primary bg-primary/5'
              : 'border-border bg-surface/50 hover:border-foreground/25',
          )}
        >
          <input
            ref={inputRef}
            type="file"
            accept={ACCEPTED_TYPES}
            onChange={onFileChange}
            className="hidden"
          />
          <div className="grid size-9 place-items-center rounded-lg bg-card text-muted-foreground shadow-[var(--shadow-1)]">
            {busy ? <Spinner /> : <Upload className="size-4" />}
          </div>
          <div className="space-y-0.5">
            <p className="text-[13px] font-medium text-foreground">
              Drop an image here, or{' '}
              <button
                type="button"
                disabled={busy}
                onClick={() => inputRef.current?.click()}
                className="text-primary underline-offset-4 hover:underline"
              >
                browse files
              </button>
            </p>
            <p className="text-[12px] text-muted-foreground">
              PNG, JPEG, WebP, GIF, or SVG. Large images are automatically resized.
            </p>
          </div>
        </div>
      ) : null}

      {activeMode === 'link' ? (
        <div className="space-y-2.5">
          <Field>
            <FieldLabel htmlFor="image-url-input">Image URL</FieldLabel>
            <div className="flex gap-2">
              <Input
                id="image-url-input"
                type="url"
                value={urlValue}
                onChange={(e) => {
                  setUrlValue(e.target.value)
                  setUrlBroken(false)
                  setLocalError('')
                }}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') {
                    e.preventDefault()
                    applyLink()
                  }
                }}
                placeholder="https://example.com/logo.png"
                spellCheck={false}
                className="flex-1"
              />
              <Button type="button" size="sm" disabled={busy || !urlValue.trim()} onClick={applyLink}>
                {busy ? <Spinner data-icon="inline-start" /> : null}
                Use link
              </Button>
            </div>
            <FieldDescription>Direct link to a publicly reachable PNG, SVG, WebP, or JPEG.</FieldDescription>
          </Field>

          {/^https?:\/\/.+/i.test(urlValue.trim()) ? (
            <div className="flex items-center gap-3 rounded-lg border border-border/80 bg-surface/50 px-3 py-2">
              <div className="grid size-10 shrink-0 place-items-center overflow-hidden rounded-lg border border-border/70 bg-card">
                {urlBroken ? (
                  <span className="text-[10px] text-muted-foreground">Error</span>
                ) : (
                  <img
                    src={urlValue.trim()}
                    alt="URL preview"
                    referrerPolicy="no-referrer"
                    onError={() => setUrlBroken(true)}
                    className="size-full object-cover"
                  />
                )}
              </div>
              <div className="min-w-0 flex-1 text-[12px]">
                {urlBroken ? (
                  <p className="text-fail">Could not load a preview from this URL. Check the link.</p>
                ) : (
                  <p className="truncate text-muted-foreground">Live preview ready</p>
                )}
              </div>
            </div>
          ) : null}
        </div>
      ) : null}

      {localError ? <FieldError>{localError}</FieldError> : null}
    </div>
  )
}
