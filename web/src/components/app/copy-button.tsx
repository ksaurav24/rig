import { Check, Copy } from 'lucide-react'
import { useEffect, useState } from 'react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'

export function CopyButton({
  value,
  getValue,
  label = 'Copy',
}: {
  value?: string
  getValue?: () => string
  label?: string
}) {
  const [copied, setCopied] = useState(false)

  useEffect(() => {
    if (!copied) return
    const t = setTimeout(() => setCopied(false), 1600)
    return () => clearTimeout(t)
  }, [copied])

  async function copy() {
    try {
      await navigator.clipboard.writeText(getValue ? getValue() : (value ?? ''))
      setCopied(true)
    } catch {
      toast.error('Copying is blocked by the browser. Select the text and copy it manually.')
    }
  }

  return (
    <Button type="button" variant="ghost" size="icon-sm" onClick={copy} aria-label={copied ? 'Copied' : label}>
      {copied ? <Check className="text-ok" /> : <Copy />}
    </Button>
  )
}
