import { useEffect } from 'react'
import { isAccentPreset, usePreferences } from '@/lib/preferences'

export interface BrandingTarget {
  orgName?: string | null
  logoUrl?: string | null
  accentColor?: string | null
}

const DEFAULT_FAVICON = '/favicon.svg'

/**
 * Mirrors organization branding (title, favicon, and org default accent) into
 * the document head and preferences context.
 */
export function useSyncBranding(branding: BrandingTarget | null | undefined) {
  const { setOrgAccent } = usePreferences()
  const orgName = branding?.orgName?.trim() || 'Rig'
  const logoUrl = branding?.logoUrl?.trim() || ''
  const accentColor = branding?.accentColor

  useEffect(() => {
    document.title = orgName
  }, [orgName])

  useEffect(() => {
    let link = document.querySelector<HTMLLinkElement>('link[rel="icon"]')
    if (!link) {
      link = document.createElement('link')
      link.rel = 'icon'
      document.head.appendChild(link)
    }
    if (logoUrl) {
      link.removeAttribute('type')
      link.href = logoUrl
    } else {
      link.type = 'image/svg+xml'
      link.href = DEFAULT_FAVICON
    }
  }, [logoUrl])

  useEffect(() => {
    if (isAccentPreset(accentColor)) {
      setOrgAccent(accentColor)
    }
  }, [accentColor, setOrgAccent])
}
