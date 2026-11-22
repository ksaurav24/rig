import { useQuery } from '@tanstack/react-query'
import { HardDrive } from 'lucide-react'
import { systemQuery } from '@/api/queries'
import { Banner, BannerDescription, BannerTitle } from '@/components/fluid/banner'
import { bytes } from '@/lib/format'

/** Deploys and backups refuse to start below 1 GiB free (see the server's `MIN_FREE_DISK`). */
const REFUSE_BELOW = 1024 ** 3
/** Warn early, while there is still room to clean up. */
const WARN_BELOW = 5 * 1024 ** 3

/** A strip under the header, shown on every page while the data disk runs low. */
export function DiskBanner() {
  const { data } = useQuery(systemQuery)
  if (!data || data.disk_total <= 0) return null
  const free = data.disk_total - data.disk_used
  if (free >= WARN_BELOW && free / data.disk_total >= 0.1) return null
  const blocked = free < REFUSE_BELOW
  return (
    <Banner
      role="status"
      variant="fixed"
      status={blocked ? 'error' : 'warning'}
      contrast="high"
      icon={HardDrive}
      className="border-b border-border/70"
    >
      <BannerTitle>Only {bytes(free)} free on the data disk.</BannerTitle>
      <BannerDescription>
        {blocked
          ? 'New deploys and backups are paused until you free up space.'
          : 'Deploys and backups pause below 1 GB. Remove old images or backups now.'}{' '}
        <code className="font-mono text-xs">{data.engine === 'podman' ? 'podman' : 'docker'} system prune</code> reclaims
        unused images.
      </BannerDescription>
    </Banner>
  )
}
