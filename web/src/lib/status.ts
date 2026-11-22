import type { Schemas } from '@/api/client'
import type { Tone } from '@/components/app/state'

type Look = { tone: Tone; label: string }

export const APP_STATUS: Record<Schemas['AppStatus'], Look> = {
  idle: { tone: 'idle', label: 'Not deployed' },
  deploying: { tone: 'busy', label: 'Deploying' },
  running: { tone: 'ok', label: 'Running' },
  stopped: { tone: 'idle', label: 'Stopped' },
  failed: { tone: 'fail', label: 'Failed' },
}

export const DATABASE_STATUS: Record<Schemas['DatabaseStatus'], Look> = {
  creating: { tone: 'busy', label: 'Starting' },
  running: { tone: 'ok', label: 'Running' },
  stopped: { tone: 'idle', label: 'Stopped' },
  failed: { tone: 'fail', label: 'Failed' },
}

export const DEPLOYMENT_STATUS: Record<Schemas['DeploymentStatus'], Look> = {
  queued: { tone: 'busy', label: 'Queued' },
  building: { tone: 'busy', label: 'Building' },
  deploying: { tone: 'busy', label: 'Starting' },
  running: { tone: 'ok', label: 'Live' },
  superseded: { tone: 'idle', label: 'Replaced' },
  failed: { tone: 'fail', label: 'Failed' },
  cancelled: { tone: 'idle', label: 'Cancelled' },
}

export function deploymentInProgress(status: Schemas['DeploymentStatus']): boolean {
  return status === 'queued' || status === 'building' || status === 'deploying'
}

const TRIGGER: Record<string, string> = {
  manual: 'Dashboard',
  api: 'API token',
  rollback: 'Rollback',
  webhook: 'Git push',
}

export function triggerLabel(trigger: string): string {
  return TRIGGER[trigger] ?? trigger
}

export const BACKUP_STATUS: Record<Schemas['BackupStatus'], Look> = {
  running: { tone: 'busy', label: 'Backing up' },
  ok: { tone: 'ok', label: 'Ready' },
  failed: { tone: 'fail', label: 'Failed' },
}
