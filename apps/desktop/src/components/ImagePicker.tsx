import { ImageUp, Link2, RotateCcw, Sparkles } from 'lucide-react'
import { useRef, useState, type ChangeEvent, type DragEvent, type FormEvent } from 'react'
import { Banner, BannerDescription } from '@rig/ui/banner'
import { Button } from '@rig/ui/button'
import { Input } from '@rig/ui/input'
import { Tabs, TabsItem } from '@rig/ui/tabs'
import { AVATAR_PRESETS } from './MediaImage'
import { cn } from '@rig/ui/lib/utils'

const MAX_UPLOAD_BYTES = 750 * 1024
const ACCEPTED_TYPES = new Set(['image/png', 'image/jpeg', 'image/webp', 'image/gif'])

interface ImagePickerProps {
  currentUrl?: string | null
  presets?: boolean
  busy?: boolean
  onUpload: (contentType: string, dataBase64: string) => Promise<void>
  onSelectUrl: (url: string) => Promise<void>
  onReset: () => Promise<void>
}

function readAsDataUrl(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => resolve(String(reader.result ?? ''))
    reader.onerror = () => reject(new Error('Could not read the selected file'))
    reader.readAsDataURL(blob)
  })
}

async function prepareUpload(file: File): Promise<{ contentType: string; dataBase64: string }> {
  const type = file.type.toLowerCase()
  if (!ACCEPTED_TYPES.has(type)) {
    throw new Error('Use a PNG, JPEG, WebP, or GIF image.')
  }
  if (file.size <= MAX_UPLOAD_BYTES) {
    const dataUrl = await readAsDataUrl(file)
    const idx = dataUrl.indexOf(',')
    return { contentType: type, dataBase64: idx >= 0 ? dataUrl.slice(idx + 1) : dataUrl }
  }
  if (type === 'image/gif') {
    throw new Error('GIF files must be 750 KB or smaller.')
  }

  // Client-side resize onto a 512px canvas so large photos upload cleanly.
  const img = await new Promise<HTMLImageElement>((resolve, reject) => {
    const el = new Image()
    const objUrl = URL.createObjectURL(file)
    el.onload = () => {
      URL.revokeObjectURL(objUrl)
      resolve(el)
    }
    el.onerror = () => {
      URL.revokeObjectURL(objUrl)
      reject(new Error('Could not decode image'))
    }
    el.src = objUrl
  })

  const maxSide = 512
  const scale = Math.min(1, maxSide / Math.max(img.width, img.height))
  const w = Math.max(1, Math.round(img.width * scale))
  const h = Math.max(1, Math.round(img.height * scale))
  const canvas = document.createElement('canvas')
  canvas.width = w
  canvas.height = h
  const ctx = canvas.getContext('2d')
  if (!ctx) throw new Error('Canvas unavailable')
  ctx.drawImage(img, 0, 0, w, h)

  const outType = type === 'image/png' ? 'image/png' : 'image/jpeg'
  const outUrl = canvas.toDataURL(outType, 0.88)
  const idx = outUrl.indexOf(',')
  const b64 = idx >= 0 ? outUrl.slice(idx + 1) : outUrl
  if (b64.length * 0.75 > MAX_UPLOAD_BYTES) {
    const jpegUrl = canvas.toDataURL('image/jpeg', 0.8)
    const jIdx = jpegUrl.indexOf(',')
    return { contentType: 'image/jpeg', dataBase64: jIdx >= 0 ? jpegUrl.slice(jIdx + 1) : jpegUrl }
  }
  return { contentType: outType, dataBase64: b64 }
}

export function ImagePicker({
  currentUrl,
  presets = false,
  busy = false,
  onUpload,
  onSelectUrl,
  onReset,
}: ImagePickerProps) {
  const modes = presets
    ? ([
        { id: 'presets', label: 'Presets', icon: Sparkles },
        { id: 'upload', label: 'Upload', icon: ImageUp },
        { id: 'link', label: 'Image URL', icon: Link2 },
      ] as const)
    : ([
        { id: 'upload', label: 'Upload', icon: ImageUp },
        { id: 'link', label: 'Image URL', icon: Link2 },
      ] as const)

  const [modeIdx, setModeIdx] = useState(0)
  const [urlInput, setUrlInput] = useState(
    currentUrl && currentUrl.startsWith('https://') ? currentUrl : '',
  )
  const [dragOver, setDragOver] = useState(false)
  const [working, setWorking] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const fileRef = useRef<HTMLInputElement>(null)

  const mode = modes[modeIdx]?.id ?? 'upload'
  const disabled = busy || working

  async function handleFile(file: File | undefined) {
    if (!file) return
    setError(null)
    setWorking(true)
    try {
      const { contentType, dataBase64 } = await prepareUpload(file)
      await onUpload(contentType, dataBase64)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setWorking(false)
      if (fileRef.current) fileRef.current.value = ''
    }
  }

  function onFileChange(e: ChangeEvent<HTMLInputElement>) {
    void handleFile(e.target.files?.[0])
  }

  function onDrop(e: DragEvent<HTMLDivElement>) {
    e.preventDefault()
    setDragOver(false)
    void handleFile(e.dataTransfer.files?.[0])
  }

  async function handleLinkSubmit(e: FormEvent) {
    e.preventDefault()
    const trimmed = urlInput.trim()
    if (!trimmed.startsWith('https://')) {
      setError('Image URL must start with https://')
      return
    }
    setError(null)
    setWorking(true)
    try {
      await onSelectUrl(trimmed)
    } catch (err) {
      setError(String(err))
    } finally {
      setWorking(false)
    }
  }

  async function handlePreset(id: string) {
    setError(null)
    setWorking(true)
    try {
      await onSelectUrl(id)
    } catch (err) {
      setError(String(err))
    } finally {
      setWorking(false)
    }
  }

  async function handleReset() {
    setError(null)
    setWorking(true)
    try {
      await onReset()
      setUrlInput('')
    } catch (err) {
      setError(String(err))
    } finally {
      setWorking(false)
    }
  }

  return (
    <div className="space-y-3.5">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <Tabs
          size="compact"
          selectedIndex={modeIdx}
          onSelect={(i) => {
            setModeIdx(i)
            setError(null)
          }}
        >
          {modes.map((m, idx) => (
            <TabsItem key={m.id} index={idx} label={m.label} icon={m.icon} />
          ))}
        </Tabs>

        {currentUrl && (
          <Button
            type="button"
            variant="ghost"
            size="compact"
            leadingIcon={RotateCcw}
            disabled={disabled}
            onClick={() => void handleReset()}
          >
            Reset to default
          </Button>
        )}
      </div>

      {error && (
        <Banner role="alert" status="error" contrast="high" size="compact">
          <BannerDescription>{error}</BannerDescription>
        </Banner>
      )}

      {mode === 'presets' && (
        <div className="grid grid-cols-4 gap-2 sm:grid-cols-6">
          {AVATAR_PRESETS.map((p) => {
            const active = currentUrl === p.id
            return (
              <button
                key={p.id}
                type="button"
                disabled={disabled}
                title={p.label}
                onClick={() => void handlePreset(p.id)}
                className={cn(
                  'group relative aspect-square overflow-hidden rounded-xl border-2 p-0.5 transition-all outline-none',
                  active
                    ? 'border-primary ring-2 ring-primary/25'
                    : 'border-border/80 hover:border-foreground/40',
                )}
              >
                <div className="size-full overflow-hidden rounded-[9px]">
                  {p.render('size-full block')}
                </div>
              </button>
            )
          })}
        </div>
      )}

      {mode === 'upload' && (
        <div
          onDragOver={(e) => {
            e.preventDefault()
            setDragOver(true)
          }}
          onDragLeave={() => setDragOver(false)}
          onDrop={onDrop}
          className={cn(
            'flex flex-col items-center justify-center gap-2 rounded-xl border border-dashed px-5 py-6 text-center transition-colors',
            dragOver
              ? 'border-primary bg-primary/8'
              : 'border-border/90 bg-surface/60 hover:border-foreground/30',
          )}
        >
          <input
            ref={fileRef}
            type="file"
            accept="image/png,image/jpeg,image/webp,image/gif"
            onChange={onFileChange}
            className="hidden"
          />
          <ImageUp className="size-5 text-muted-foreground" />
          <div className="text-[13px] font-medium text-foreground">
            Drop an image here, or choose a file
          </div>
          <p className="text-[12px] text-muted-foreground">
            PNG, JPEG, WebP, or GIF · large images are automatically resized to 512px
          </p>
          <Button
            type="button"
            variant="secondary"
            size="compact"
            loading={disabled}
            onClick={() => fileRef.current?.click()}
            className="mt-1"
          >
            Choose Image…
          </Button>
        </div>
      )}

      {mode === 'link' && (
        <form onSubmit={handleLinkSubmit} className="flex flex-wrap items-center gap-2">
          <Input
            type="url"
            required
            value={urlInput}
            onChange={(e) => setUrlInput(e.target.value)}
            placeholder="https://example.com/logo.png"
            className="min-w-[220px] flex-1 font-mono"
          />
          <Button type="submit" variant="primary" size="compact" loading={disabled}>
            Use URL
          </Button>
        </form>
      )}
    </div>
  )
}
