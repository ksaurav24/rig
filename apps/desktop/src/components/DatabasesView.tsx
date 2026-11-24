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
  X,
} from 'lucide-react'
import { useEffect, useState, type FormEvent } from 'react'
import { toast } from 'sonner'
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
  { engine: 'redis', label: 'Redis', versions: ['7', '6'], port: 6379 },
  { engine: 'mysql', label: 'MySQL', versions: ['8.4', '8.0'], port: 3306 },
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

  const engines = enginesQuery.data && enginesQuery.data.length > 0 ? enginesQuery.data : FALLBACK_ENGINES
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
    <div className="space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h1 className="text-lg font-semibold tracking-tight text-fg">Managed Databases</h1>
          <p className="text-xs text-muted">
            Provision PostgreSQL, Redis, MySQL, or MongoDB instances with automatic credentials and
            backups.
          </p>
        </div>
        <button
          type="button"
          onClick={() => setCreateOpen(true)}
          className="inline-flex items-center gap-1.5 rounded-lg bg-emerald-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-emerald-500"
        >
          <Plus className="size-3.5" />
          Create Database
        </button>
      </div>

      {databases.length === 0 ? (
        <div className="flex flex-col items-center justify-center rounded-xl border border-line bg-surface p-10 text-center">
          <div className="mb-3 flex size-10 items-center justify-center rounded-xl bg-sky-500/15 text-sky-400">
            <DbIcon className="size-5" />
          </div>
          <div className="text-sm font-medium text-fg">No managed databases yet</div>
          <p className="mt-1 max-w-sm text-xs text-muted">
            Spin up a managed PostgreSQL, Redis, MySQL, or MongoDB instance on this server in one
            click.
          </p>
          <button
            type="button"
            onClick={() => setCreateOpen(true)}
            className="mt-4 inline-flex items-center gap-1.5 rounded-lg bg-emerald-600 px-3.5 py-2 text-xs font-medium text-white hover:bg-emerald-500"
          >
            <Plus className="size-3.5" />
            Create Your First Database
          </button>
        </div>
      ) : (
        <div className="space-y-3">
          {databases.map((db) => {
            const creds = revealed[db.id]
            const backups = expandedBackups[db.id]
            return (
              <div key={db.id} className="rounded-xl border border-line bg-surface p-4 text-xs">
                <div className="flex flex-wrap items-center justify-between gap-3">
                  <div className="flex items-center gap-3">
                    <div className="flex size-9 items-center justify-center rounded-lg bg-sky-500/15 text-sky-400">
                      <DbIcon className="size-4" />
                    </div>
                    <div>
                      <div className="flex items-center gap-2">
                        <span className="text-sm font-semibold text-fg">{db.name}</span>
                        <span
                          className={`rounded px-1.5 py-0.5 font-mono text-[10px] uppercase ${
                            db.status === 'running'
                              ? 'bg-emerald-500/15 text-emerald-300'
                              : db.status === 'failed'
                                ? 'bg-rose-500/15 text-rose-300'
                                : 'bg-surface-2 text-muted'
                          }`}
                        >
                          {db.status}
                        </span>
                        <span className="rounded bg-bg px-1.5 py-0.5 text-[10px] text-muted">
                          {db.project_name}
                        </span>
                      </div>
                      <div className="mt-0.5 font-mono text-[11px] text-muted">
                        {db.engine} {db.version} · {db.host}:{db.port}
                        {db.db_name ? ` / ${db.db_name}` : ''}
                        {db.host_port ? ` · host:${db.host_port}` : ''}
                      </div>
                    </div>
                  </div>

                  <div className="flex flex-wrap items-center gap-1.5">
                    <button
                      type="button"
                      onClick={() => void reveal(db)}
                      className="inline-flex items-center gap-1 rounded-lg border border-line bg-surface-2 px-2.5 py-1.5 text-xs text-fg hover:bg-bg"
                    >
                      <KeyRound className="size-3.5" />
                      {creds ? 'Hide Credentials' : 'Credentials'}
                    </button>
                    <button
                      type="button"
                      onClick={() => void toggleBackups(db)}
                      className="inline-flex items-center gap-1 rounded-lg border border-line bg-surface-2 px-2.5 py-1.5 text-xs text-fg hover:bg-bg"
                    >
                      <Archive className="size-3.5" />
                      Backups
                      {backups ? <ChevronUp className="size-3" /> : <ChevronDown className="size-3" />}
                    </button>
                    {db.status === 'running' ? (
                      <button
                        type="button"
                        disabled={busyId === db.id}
                        onClick={() => void toggleDb(db, 'stop')}
                        className="inline-flex items-center gap-1 rounded-lg border border-line bg-surface-2 px-2.5 py-1.5 text-xs text-rose-300 hover:bg-rose-500/10"
                      >
                        <Square className="size-3" />
                        Stop
                      </button>
                    ) : (
                      <button
                        type="button"
                        disabled={busyId === db.id}
                        onClick={() => void toggleDb(db, 'start')}
                        className="inline-flex items-center gap-1 rounded-lg bg-emerald-600 px-2.5 py-1.5 text-xs font-medium text-white hover:bg-emerald-500"
                      >
                        <Play className="size-3" />
                        Start
                      </button>
                    )}
                    <button
                      type="button"
                      disabled={busyId === db.id}
                      onClick={() => void handleDeleteDb(db)}
                      title="Delete database"
                      className="rounded-lg border border-line bg-surface-2 p-1.5 text-muted hover:bg-rose-500/15 hover:text-rose-300"
                    >
                      <Trash2 className="size-3.5" />
                    </button>
                  </div>
                </div>

                {db.error && (
                  <div className="mt-2.5 rounded-lg border border-rose-500/30 bg-rose-500/10 px-3 py-1.5 text-[11px] text-rose-300">
                    {db.error}
                  </div>
                )}

                {creds && (
                  <div className="mt-3 space-y-2 rounded-lg border border-line bg-bg p-3 font-mono text-[11px]">
                    <div className="flex items-center gap-2">
                      <span className="w-20 shrink-0 text-muted">URL</span>
                      <span className="min-w-0 flex-1 truncate text-emerald-300">{creds.url}</span>
                      <button
                        type="button"
                        onClick={() => {
                          void navigator.clipboard.writeText(creds.url)
                          toast.success('Copied connection URL')
                        }}
                        className="rounded p-1 text-muted hover:bg-surface-2 hover:text-fg"
                      >
                        <Copy className="size-3.5" />
                      </button>
                    </div>
                    <div className="flex items-center gap-2">
                      <span className="w-20 shrink-0 text-muted">Password</span>
                      <span className="min-w-0 flex-1 truncate text-fg">{creds.password}</span>
                      <button
                        type="button"
                        onClick={() => {
                          void navigator.clipboard.writeText(creds.password)
                          toast.success('Copied password')
                        }}
                        className="rounded p-1 text-muted hover:bg-surface-2 hover:text-fg"
                      >
                        <Copy className="size-3.5" />
                      </button>
                    </div>
                    <div className="flex items-center gap-4 text-[10px] text-muted">
                      <span>user: {db.username}</span>
                      <span>host: {db.host}</span>
                      <span>port: {db.port}</span>
                      {db.db_name && <span>db: {db.db_name}</span>}
                    </div>
                  </div>
                )}

                {backups && (
                  <div className="mt-3 overflow-hidden rounded-lg border border-line bg-bg">
                    <div className="flex items-center justify-between border-b border-line px-3 py-2">
                      <span className="font-medium text-fg">Backups ({backups.length})</span>
                      <button
                        type="button"
                        disabled={loadingBackupDb === db.id}
                        onClick={() => void triggerBackup(db)}
                        className="inline-flex items-center gap-1 rounded-md bg-emerald-600 px-2.5 py-1 text-[11px] font-medium text-white hover:bg-emerald-500 disabled:opacity-50"
                      >
                        <Plus className="size-3" />
                        {loadingBackupDb === db.id ? 'Working…' : 'Create Backup Now'}
                      </button>
                    </div>
                    {backups.length === 0 ? (
                      <div className="px-3 py-4 text-center text-[11px] text-muted">
                        No backups yet. Click &ldquo;Create Backup Now&rdquo; to snapshot this
                        database.
                      </div>
                    ) : (
                      <div className="divide-y divide-line">
                        {backups.map((b) => (
                          <div
                            key={b.id}
                            className="flex flex-wrap items-center justify-between gap-2 px-3 py-2 text-[11px]"
                          >
                            <div className="flex items-center gap-2 font-mono">
                              <span className="text-fg">{b.id.slice(-8)}</span>
                              <span
                                className={`rounded px-1.5 py-0.5 text-[10px] uppercase ${
                                  b.status === 'succeeded' || b.status === 'completed'
                                    ? 'bg-emerald-500/15 text-emerald-300'
                                    : b.status === 'failed'
                                      ? 'bg-rose-500/15 text-rose-300'
                                      : 'bg-surface-2 text-muted'
                                }`}
                              >
                                {b.status}
                              </span>
                              {b.size_bytes != null && (
                                <span className="text-muted">{formatBytes(b.size_bytes)}</span>
                              )}
                              <span className="text-muted">{relativeTime(b.created_at)}</span>
                            </div>
                            <div className="flex items-center gap-1.5">
                              {b.restorable && (
                                <button
                                  type="button"
                                  onClick={async () => {
                                    try {
                                      await desktopApi.restoreBackup(serverId, b.id)
                                      toast.success('Backup restored')
                                    } catch (err) {
                                      toast.error(String(err))
                                    }
                                  }}
                                  className="inline-flex items-center gap-1 rounded border border-line bg-surface-2 px-2 py-0.5 text-[11px] text-fg hover:bg-surface"
                                >
                                  <RotateCcw className="size-3" />
                                  Restore
                                </button>
                              )}
                              <button
                                type="button"
                                onClick={async () => {
                                  try {
                                    await desktopApi.deleteBackup(serverId, b.id)
                                    toast.success('Backup deleted')
                                    await reloadBackups(db.id)
                                  } catch (err) {
                                    toast.error(String(err))
                                  }
                                }}
                                className="rounded p-1 text-muted hover:bg-rose-500/15 hover:text-rose-300"
                              >
                                <Trash2 className="size-3" />
                              </button>
                            </div>
                          </div>
                        ))}
                      </div>
                    )}
                  </div>
                )}
              </div>
            )
          })}
        </div>
      )}

      {/* Create Database Modal */}
      {createOpen && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4">
          <div className="w-full max-w-md overflow-hidden rounded-2xl border border-line bg-surface shadow-2xl">
            <div className="flex items-center justify-between border-b border-line px-5 py-4">
              <div className="flex items-center gap-2.5">
                <div className="flex size-8 items-center justify-center rounded-lg bg-sky-500/15 text-sky-400">
                  <DbIcon className="size-4" />
                </div>
                <div>
                  <h2 className="text-sm font-semibold text-fg">Create Managed Database</h2>
                  <p className="text-xs text-muted">
                    Runs isolated in a managed container on your server
                  </p>
                </div>
              </div>
              <button
                type="button"
                onClick={() => setCreateOpen(false)}
                className="rounded-lg p-1.5 text-muted hover:bg-surface-2 hover:text-fg"
              >
                <X className="size-4" />
              </button>
            </div>

            <form onSubmit={handleCreate} className="space-y-4 p-5 text-xs">
              <div>
                <label className="mb-1.5 block font-medium text-fg">Engine</label>
                <div className="grid grid-cols-2 gap-2">
                  {engines.map((eng) => (
                    <button
                      key={eng.engine}
                      type="button"
                      onClick={() => {
                        setEngine(eng.engine)
                        setVersion(eng.versions[0] ?? '')
                      }}
                      className={`flex items-center justify-between rounded-xl border px-3 py-2.5 text-left transition-colors ${
                        engine === eng.engine
                          ? 'border-emerald-500 bg-emerald-500/10 text-fg'
                          : 'border-line bg-bg text-muted hover:text-fg'
                      }`}
                    >
                      <span className="font-medium">{eng.label}</span>
                      <span className="font-mono text-[10px] opacity-75">:{eng.port}</span>
                    </button>
                  ))}
                </div>
              </div>

              <div className="grid grid-cols-2 gap-3">
                <div>
                  <label className="mb-1 block font-medium text-fg">Database Name</label>
                  <input
                    type="text"
                    required
                    value={dbName}
                    onChange={(e) => setDbName(e.target.value)}
                    placeholder="app-db, cache…"
                    className="w-full rounded-lg border border-line bg-bg px-3 py-2 text-xs text-fg outline-none focus:border-emerald-500"
                  />
                </div>
                <div>
                  <label className="mb-1 block font-medium text-fg">Version</label>
                  <select
                    value={version}
                    onChange={(e) => setVersion(e.target.value)}
                    className="w-full rounded-lg border border-line bg-bg px-3 py-2 font-mono text-xs text-fg outline-none focus:border-emerald-500"
                  >
                    {(activeEngine?.versions ?? []).map((v) => (
                      <option key={v} value={v}>
                        {v}
                      </option>
                    ))}
                  </select>
                </div>
              </div>

              <div>
                <label className="mb-1 block font-medium text-fg">Project</label>
                <select
                  value={projectId}
                  onChange={(e) => setProjectId(e.target.value)}
                  className="w-full rounded-lg border border-line bg-bg px-3 py-2 text-xs text-fg outline-none focus:border-emerald-500"
                >
                  {projects.map((p) => (
                    <option key={p.id} value={p.id}>
                      {p.name}
                    </option>
                  ))}
                  <option value="__new__">+ Create new project…</option>
                </select>
              </div>

              {projectId === '__new__' && (
                <div>
                  <label className="mb-1 block font-medium text-fg">New Project Name</label>
                  <input
                    type="text"
                    required
                    value={newProjectName}
                    onChange={(e) => setNewProjectName(e.target.value)}
                    placeholder="Production"
                    className="w-full rounded-lg border border-line bg-bg px-3 py-2 text-xs text-fg outline-none focus:border-emerald-500"
                  />
                </div>
              )}

              <div className="flex items-center justify-end gap-2 border-t border-line pt-4">
                <button
                  type="button"
                  onClick={() => setCreateOpen(false)}
                  className="rounded-lg px-3 py-1.5 text-muted hover:bg-surface-2 hover:text-fg"
                >
                  Cancel
                </button>
                <button
                  type="submit"
                  disabled={creating || !dbName.trim()}
                  className="inline-flex items-center gap-1.5 rounded-lg bg-emerald-600 px-4 py-2 font-semibold text-white hover:bg-emerald-500 disabled:opacity-50"
                >
                  <Plus className="size-3.5" />
                  {creating ? 'Creating…' : 'Create Database'}
                </button>
              </div>
            </form>
          </div>
        </div>
      )}
    </div>
  )
}
