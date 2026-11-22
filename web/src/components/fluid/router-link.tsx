import { useRouter } from '@tanstack/react-router'
import type { AnchorHTMLAttributes } from 'react'

/** Stand-in for `next/link` in the Fluid registry files: a plain anchor that
 *  hands same-origin, unmodified left clicks to the TanStack router. */
export default function Link({ href, onClick, target, ...rest }: AnchorHTMLAttributes<HTMLAnchorElement> & { href: string }) {
  const router = useRouter()
  return (
    <a
      href={href}
      target={target}
      onClick={(e) => {
        onClick?.(e)
        if (e.defaultPrevented || target || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return
        if (/^[a-z][a-z0-9+.-]*:/i.test(href)) return
        e.preventDefault()
        void router.navigate({ to: href })
      }}
      {...rest}
    />
  )
}
