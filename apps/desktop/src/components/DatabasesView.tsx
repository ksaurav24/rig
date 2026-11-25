import { useQuery } from '@tanstack/react-query'
import {
  Archive,
  ChevronDown,
  ChevronUp,
  Copy,
  Database as DbIcon,
  KeyRound,
  Play,
  Plus,
  RotateCcw,
  Square,
  Trash2,
} from 'lucide-react'
import { useEffect, useState, type FormEvent } from 'react'
import { toast } from 'sonner'
import { Page, Panel } from '@rig/ui/app/page'
import { EmptyState, Status, statusTone } from '@rig/ui/app/state'
import { Badge } from '@rig/ui/badge'
import { Banner, BannerDescription } from '@rig/ui/banner'
import { Button } from '@rig/ui/button'
import { ConfirmDialog } from '@rig/ui/confirm-dialog'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@rig/ui/dialog'
import { Field, FieldLabel } from '@rig/ui/field'
import { Input } from '@rig/ui/input'
import { Select, SelectContent, SelectItem, SelectTrigger } from '@rig/ui/select'
import { formatBytes, relativeTime } from '@/lib/format'
import { desktopApi } from '@/lib/ipc'
import type {
  Backup,
  Database,
  DatabaseCredentials,
  DatabaseEngine,
  DatabaseEngineInfo,
  Project,
} from '@/lib/types'

interface DatabasesViewProps {
  serverId: string
  projects: Project[]
  databases: Database[]
  autoOpenCreate?: boolean
  onConsumeAutoCreate?: () => void
  onRefresh: () => void
}

const FALLBACK_ENGINES: DatabaseEngineInfo[] = [
  { engine: 'postgres', label: 'PostgreSQL', versions: ['17', '16', '15'], port: 5432 },
  { engine: 'mysql', label: 'MySQL', versions: ['8.4', '8.0'], port: 3306 },
  { engine: 'mariadb', label: 'MariaDB', versions: ['11.4', '10.11'], port: 3306 },
  { engine: 'redis', label: 'Redis', versions: ['7', '6'], port: 6379 },
  { engine: 'mongodb', label: 'MongoDB', versions: ['7', '6'], port: 27017 },
]

export function DatabasesView({
  serverId,
  projects,
  databases,
  autoOpenCreate,
  onConsumeAutoCreate,
  onRefresh,
}: DatabasesViewProps) {
  const [revealed, setRevealed] = useState<Record<string, DatabaseCredentials>>({})
  const [expandedBackups, setExpandedBackups] = useState<Record<string, Backup[]>>({})
  const [loadingBackupDb, setLoadingBackupDb] = useState<string | null>(null)
  const [busyId, setBusyId] = useState<string | null>(null)
  const [createOpen, setCreateOpen] = useState(false)
  const [confirmState, setConfirmState] = useState<{
    title: string
    description: string
    target?: string
    confirmLabel: string
    tone: 'danger' | 'warning'
    action: () => Promise<void>
  } | null>(null)

  // Create Database form state
  const [projectId, setProjectId] = useState<string>(projects[0]?.id ?? '__new__')
  const [newProjectName, setNewProjectName] = useState('Default')
  const [dbName, setDbName] = useState('')
  const [engine, setEngine] = useState<DatabaseEngine>('postgres')
  const [version, setVersion] = useState('17')
  const [creating, setCreating] = useState(false)

  const enginesQuery = useQuery({
    queryKey: ['desktop', serverId, 'db-engines'],
    queryFn: () => desktopApi.listDatabaseEngines(serverId),
    enabled: createOpen,
  })

  const engines =
    enginesQuery.data && enginesQuery.data.length > 0 ? enginesQuery.data : FALLBACK_ENGINES
  const activeEngine = engines.find((e) => e.engine === engine) ?? engines[0]

  useEffect(() => {
    if (autoOpenCreate) {
      setCreateOpen(true)
      onConsumeAutoCreate?.()
    }
  }, [autoOpenCreate, onConsumeAutoCreate])

  useEffect(() => {
    if (createOpen) {
      setProjectId(projects[0]?.id ?? '__new__')
    }
  }, [createOpen, projects])

  useEffect(() => {
    if (activeEngine && !activeEngine.versions.includes(version)) {
      setVersion(activeEngine.versions[0] ?? '')
    }
  }, [activeEngine, version])

  async function toggleDb(db: Database, action: 'start' | 'stop') {
    setBusyId(db.id)
    try {
      await desktopApi.databaseAction(serverId, db.id, action)
      toast.success(`${action === 'start' ? 'Started' : 'Stopped'} ${db.name}`)
      onRefresh()
    } catch (e) {
      toast.error(String(e))
    } finally {
      setBusyId(null)
    }
  }

  async function reveal(db: Database) {
    if (revealed[db.id]) {
      setRevealed((prev) => {
        const next = { ...prev }
        delete next[db.id]
        return next
      })
      return
    }
    try {
      const creds = await desktopApi.revealDatabase(serverId, db.id)
      setRevealed((prev) => ({ ...prev, [db.id]: creds }))
    } catch (e) {
      toast.error(String(e))
    }
  }

  async function toggleBackups(db: Database) {
    if (expandedBackups[db.id]) {
      setExpandedBackups((prev) => {
        const next = { ...prev }
        delete next[db.id]
        return next
      })
      return
    }
    await reloadBackups(db.id)
  }

  async function reloadBackups(dbId: string) {
    setLoadingBackupDb(dbId)
    try {
      const list = await desktopApi.listBackups(serverId, dbId)
      setExpandedBackups((prev) => ({ ...prev, [dbId]: list }))
    } catch (e) {
      toast.error(String(e))
    } finally {
      setLoadingBackupDb(null)
    }
  }

  async function triggerBackup(db: Database) {
    setLoadingBackupDb(db.id)
    try {
      await desktopApi.createBackup(serverId, db.id)
      toast.success(`Backup started for ${db.name}`)
      await reloadBackups(db.id)
    } catch (e) {
      toast.error(String(e))
    } finally {
      setLoadingBackupDb(null)
    }
  }

  async function handleDeleteDb(db: Database) {
    setBusyId(db.id)
    try {
      await desktopApi.deleteDatabase(serverId, db.id)
      toast.success(`Deleted database ${db.name}`)
      onRefresh()
    } catch (e) {
      toast.error(String(e))
    } finally {
      setBusyId(null)
    }
  }

  async function handleCreate(e: FormEvent) {
    e.preventDefault()
    if (!dbName.trim()) return
    setCreating(true)
    try {
      let targetProjectId = projectId
      if (targetProjectId === '__new__') {
        const proj = await desktopApi.createProject(
          serverId,
          newProjectName.trim() || 'Default',
          '',
        )
        targetProjectId = proj.id
      }
      const created = await desktopApi.createDatabase(serverId, {
        project_id: targetProjectId,
        name: dbName.trim(),
        engine,
        version: version || null,
      })
      toast.success(`Provisioning ${created.engine} database "${created.name}"`)
      setDbName('')
      setCreateOpen(false)
      onRefresh()
    } catch (err) {
      toast.error(String(err))
    } finally {
      setCreating(false)
    }
  }

  return (
    <Page
      title="Managed Databases"
      description="Provision PostgreSQL, MySQL, MariaDB, Redis, or MongoDB instances with automatic credentials and one-click backups."
      actions={
        <Button type="button" variant="primary" leadingIcon={Plus} onClick={() => setCreateOpen(true)}>
          Create Database
        </Button>
      }
    >
      {databases.length === 0 ? (
        <EmptyState
          icon={DbIcon}
          title="No managed databases yet"
          action={
            <Button
              type="button"
              variant="primary"
              leadingIcon={Plus}
              onClick={() => setCreateOpen(true)}
            >
              Create Your First Database
            </Button>
          }
        >
          Spin up a managed PostgreSQL, MySQL, MariaDB, Redis, or MongoDB instance on this server in
          one click.
        </EmptyState>
      ) : (
        <div className="space-y-4">
          {databases.map((db) => {
            const creds = revealed[db.id]
            const backups = expandedBackups[db.id]
            return (
              <Panel key={db.id}>
                <div className="flex flex-wrap items-center justify-between gap-4">
                  <div className="flex items-center gap-3.5">
                    <div className="flex size-10 items-center justify-center rounded-xl bg-hover text-busy">
                      <DbIcon className="size-5" strokeWidth={1.7} />
                    </div>
                    <div>
                      <div className="flex flex-wrap items-center gap-2">
                        <span className="text-[15px] font-semibold text-foreground">{db.name}</span>
                        <Status tone={statusTone(db.status)} size="compact" variant="dot">
                          {db.status}
                        </Status>
                        <Badge variant="solid" color="gray" size="compact">
                          {db.project_name}
                        </Badge>
                      </div>
                      <div className="mt-0.5 font-mono text-[12.5px] text-muted-foreground">
                        {db.engine} {db.version} · {db.host}:{db.port}
                        {db.db_name ? ` / ${db.db_name}` : ''}
                        {db.host_port ? ` · host:${db.host_port}` : ''}
                      </div>
                    </div>
                  </div>

                  <div className="flex flex-wrap items-center gap-2">
                    <Button
                      type="button"
                      variant="tertiary"
                      size="compact"
                      leadingIcon={KeyRound}
                      active={Boolean(creds)}
                      onClick={() => void reveal(db)}
                    >
                      {creds ? 'Hide Credentials' : 'Credentials'}
                    </Button>
                    <Button
                      type="button"
                      variant="tertiary"
                      size="compact"
                      leadingIcon={Archive}
                      trailingIcon={backups ? ChevronUp : ChevronDown}
                      active={Boolean(backups)}
                      onClick={() => void toggleBackups(db)}
                    >
                      Backups
                    </Button>
                    {db.status === 'running' ? (
                      <Button
                        type="button"
                        variant="destructive"
                        size="compact"
                        leadingIcon={Square}
                        loading={busyId === db.id}
                        disabled={busyId !== null}
                        onClick={() =>
                          setConfirmState({
                            title: 'Stop Database',
                            description:
                              'Stops the database container immediately. Connected applications will lose database access until it is started again.',
                            target: `${db.name} (${db.engine} ${db.version})`,
                            confirmLabel: 'Stop Database',
                            tone: 'warning',
                            action: () => toggleDb(db, 'stop'),
                          })
                        }
                      >
                        Stop
                      </Button>
                    ) : (
                      <Button
                        type="button"
                        variant="primary"
                        size="compact"
                        leadingIcon={Play}
                        loading={busyId === db.id}
                        disabled={busyId !== null}
                        onClick={() => void toggleDb(db, 'start')}
                      >
                        Start
                      </Button>
                    )}
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon-compact"
                      disabled={busyId !== null}
                      onClick={() =>
                        setConfirmState({
                          title: 'Delete Database',
                          description:
                            'Permanently stops and deletes this database container, its data volume, and all stored records. This cannot be undone.',
                          target: `${db.name} (${db.engine} ${db.version})`,
                          confirmLabel: 'Delete Database',
                          tone: 'danger',
                          action: () => handleDeleteDb(db),
                        })
                      }
                      aria-label={`Delete ${db.name}`}
                      title="Delete database"
                      className="text-muted-foreground hover:text-fail"
                    >
                      <Trash2 />
                    </Button>
                  </div>
                </div>

                {db.error && (
                  <Banner status="error" contrast="high" size="compact" className="mt-3.5">
                    <BannerDescription>{db.error}</BannerDescription>
                  </Banner>
                )}

                {creds && (
                  <div className="mt-4 space-y-2.5 rounded-lg border border-border/80 bg-surface p-3.5 font-mono text-[12.5px]">
                    <div className="flex items-center gap-3">
                      <span className="w-20 shrink-0 text-muted-foreground">URL</span>
                      <span className="min-w-0 flex-1 truncate text-ok">{creds.url}</span>
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon-compact"
                        aria-label="Copy connection URL"
                        onClick={() => {
                          void navigator.clipboard.writeText(creds.url)
                          toast.success('Copied connection URL')
                        }}
                      >
                        <Copy />
                      </Button>
                    </div>
                    <div className="flex items-center gap-3">
                      <span className="w-20 shrink-0 text-muted-foreground">Password</span>
                      <span className="min-w-0 flex-1 truncate text-foreground">
                        {creds.password}
                      </span>
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon-compact"
                        aria-label="Copy password"
                        onClick={() => {
                          void navigator.clipboard.writeText(creds.password)
                          toast.success('Copied password')
                        }}
                      >
                        <Copy />
                      </Button>
                    </div>
                    <div className="flex flex-wrap items-center gap-4 border-t border-border/60 pt-2 text-[11.5px] text-muted-foreground">
                      <span>user: {db.username}</span>
                      <span>host: {db.host}</span>
                      <span>port: {db.port}</span>
                      {db.db_name && <span>db: {db.db_name}</span>}
                    </div>
                  </div>
                )}

                {backups && (
                  <div className="mt-4 overflow-hidden rounded-lg border border-border/80 bg-surface">
                    <div className="flex items-center justify-between border-b border-border/80 px-4 py-2.5">
                      <span className="text-[13px] font-medium text-foreground">
                        Backups ({backups.length})
                      </span>
                      <Button
                        type="button"
                        variant="primary"
                        size="compact"
                        leadingIcon={Plus}
                        loading={loadingBackupDb === db.id}
                        onClick={() => void triggerBackup(db)}
                      >
                        Create Backup Now
                      </Button>
                    </div>
                    {backups.length === 0 ? (
                      <div className="px-4 py-6 text-center text-[13px] text-muted-foreground">
                        No backups yet. Click &ldquo;Create Backup Now&rdquo; to snapshot this
                        database.
                      </div>
                    ) : (
                      <div className="divide-y divide-border/80">
                        {backups.map((b) => (
                          <div
                            key={b.id}
                            className="flex flex-wrap items-center justify-between gap-2 px-4 py-2.5 text-[13px]"
                          >
                            <div className="flex flex-wrap items-center gap-2.5 font-mono">
                              <span className="text-foreground">{b.id.slice(-8)}</span>
                              <Status tone={statusTone(b.status)} size="compact">
                                {b.status}
                              </Status>
                              {b.size_bytes != null && (
                                <span className="text-[12px] text-muted-foreground">
                                  {formatBytes(b.size_bytes)}
                                </span>
                              )}
                              <span className="text-[12px] text-muted-foreground tabular-nums">
                                {relativeTime(b.created_at)}
                              </span>
                            </div>
                            <div className="flex items-center gap-1.5">
                              {b.restorable && (
                                <Button
                                  type="button"
                                  variant="tertiary"
                                  size="compact"
                                  leadingIcon={RotateCcw}
                                  onClick={() =>
                                    setConfirmState({
                                      title: 'Restore Database Backup',
                                      description:
                                        'Overwrites the current database contents with this backup snapshot. Any data written since this backup was created will be lost.',
                                      target: `${db.name} ← backup ${b.id.slice(-8)}`,
                                      confirmLabel: 'Restore Backup',
                                      tone: 'warning',
                                      action: async () => {
                                        try {
                                          await desktopApi.restoreBackup(serverId, b.id)
                                          toast.success('Backup restored')
                                        } catch (err) {
                                          toast.error(String(err))
                                        }
                                      },
                                    })
                                  }
                                >
                                  Restore
                                </Button>
                              )}
                              <Button
                                type="button"
                                variant="ghost"
                                size="icon-compact"
                                aria-label="Delete backup"
                                onClick={() =>
                                  setConfirmState({
                                    title: 'Delete Backup',
                                    description:
                                      'Permanently deletes this backup snapshot from the server. This cannot be undone.',
                                    target: `Backup ${b.id.slice(-8)} (${db.name})`,
                                    confirmLabel: 'Delete Backup',
                                    tone: 'danger',
                                    action: async () => {
                                      try {
                                        await desktopApi.deleteBackup(serverId, b.id)
                                        toast.success('Backup deleted')
                                        await reloadBackups(db.id)
                                      } catch (err) {
                                        toast.error(String(err))
                                      }
                                    },
                                  })
                                }
                                className="text-muted-foreground hover:text-fail"
                              >
                                <Trash2 />
                              </Button>
                            </div>
                          </div>
                        ))}
                      </div>
                    )}
                  </div>
                )}
              </Panel>
            )
          })}
        </div>
      )}

      {/* Fluid Create Database Dialog */}
      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent size="lg">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2.5">
              <DbIcon className="size-4.5 text-busy" />
              Create Managed Database
            </DialogTitle>
            <DialogDescription>
              Runs isolated in a managed container on your server with generated credentials.
            </DialogDescription>
          </DialogHeader>

          <form onSubmit={handleCreate} className="space-y-4">
            <div className="grid grid-cols-1 gap-3.5 sm:grid-cols-2">
              <Field>
                <FieldLabel>Engine</FieldLabel>
                <Select
                  value={engine}
                  onValueChange={(v) => {
                    const nextEngine = v as DatabaseEngine
                    setEngine(nextEngine)
                    const info = engines.find((e) => e.engine === nextEngine)
                    if (info?.versions[0]) setVersion(info.versions[0])
                  }}
                >
                  <SelectTrigger className="w-full" placeholder="Select engine" />
                  <SelectContent>
                    {engines.map((eng, idx) => (
                      <SelectItem key={eng.engine} index={idx} value={eng.engine}>
                        {`${eng.label} (:${eng.port})`}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </Field>

              <Field>
                <FieldLabel>Version</FieldLabel>
                <Select value={version} onValueChange={setVersion}>
                  <SelectTrigger className="w-full font-mono" placeholder="Select version" />
                  <SelectContent>
                    {(activeEngine?.versions ?? []).map((v, idx) => (
                      <SelectItem key={v} index={idx} value={v} className="font-mono">
                        {v}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </Field>
            </div>

            <Field>
              <FieldLabel htmlFor="db-create-name">Database Name</FieldLabel>
              <Input
                id="db-create-name"
                type="text"
                required
                autoFocus
                value={dbName}
                onChange={(e) => setDbName(e.target.value)}
                placeholder={`${engine}-main`}
              />
            </Field>

            <Field>
              <FieldLabel>Project</FieldLabel>
              <Select value={projectId} onValueChange={setProjectId}>
                <SelectTrigger className="w-full" placeholder="Select project" />
                <SelectContent>
                  {projects.map((p, idx) => (
                    <SelectItem key={p.id} index={idx} value={p.id}>
                      {p.name}
                    </SelectItem>
                  ))}
                  <SelectItem index={projects.length} value="__new__">
                    + Create new project…
                  </SelectItem>
                </SelectContent>
              </Select>
            </Field>

            {projectId === '__new__' && (
              <Field>
                <FieldLabel htmlFor="db-new-proj">New Project Name</FieldLabel>
                <Input
                  id="db-new-proj"
                  type="text"
                  required
                  value={newProjectName}
                  onChange={(e) => setNewProjectName(e.target.value)}
                  placeholder="Production"
                />
              </Field>
            )}

            <DialogFooter>
              <Button type="button" variant="ghost" onClick={() => setCreateOpen(false)}>
                Cancel
              </Button>
              <Button
                type="submit"
                variant="primary"
                leadingIcon={Plus}
                loading={creating}
                disabled={!dbName.trim()}
              >
                Create Database
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      <ConfirmDialog
        open={Boolean(confirmState)}
        onOpenChange={(open) => {
          if (!open) setConfirmState(null)
        }}
        title={confirmState?.title ?? ''}
        description={confirmState?.description ?? ''}
        target={confirmState?.target}
        confirmLabel={confirmState?.confirmLabel}
        tone={confirmState?.tone}
        onConfirm={async () => {
          await confirmState?.action()
        }}
      />
    </Page>
  )
}
