import { Pencil, Trash2 } from 'lucide-react'
import { useEffect, useState, type FormEvent } from 'react'
import { toast } from 'sonner'
import { Banner, BannerDescription } from '@rig/ui/banner'
import { Button } from '@rig/ui/button'
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
import { desktopApi } from '@/lib/ipc'
import type { Database, Project } from '@/lib/types'
import { cn } from '@rig/ui/lib/utils'

interface EditProjectDialogProps {
  serverId: string
  project: Project | null
  onClose: () => void
  onSaved: (project: Project) => void
}

export function EditProjectDialog({
  serverId,
  project,
  onClose,
  onSaved,
}: EditProjectDialogProps) {
  const [name, setName] = useState('')
  const [description, setDescription] = useState('')
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (!project) return
    setName(project.name)
    setDescription(project.description ?? '')
    setError(null)
  }, [project])

  async function handleSubmit(e: FormEvent) {
    e.preventDefault()
    if (!project || !name.trim()) return
    setSaving(true)
    setError(null)
    try {
      const updated = await desktopApi.updateProject(
        serverId,
        project.id,
        name.trim(),
        description.trim(),
      )
      toast.success(`Updated project "${updated.name}"`)
      onSaved(updated)
      onClose()
    } catch (err) {
      setError(String(err))
    } finally {
      setSaving(false)
    }
  }

  return (
    <Dialog open={Boolean(project)} onOpenChange={(open) => !open && onClose()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Pencil className="size-4 text-primary" />
            Edit Project
          </DialogTitle>
          <DialogDescription>
            Update this project&rsquo;s display name and description. Existing apps and databases stay
            attached.
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="space-y-4">
          {error && (
            <Banner role="alert" status="error" contrast="high" size="compact">
              <BannerDescription>{error}</BannerDescription>
            </Banner>
          )}
          <Field>
            <FieldLabel htmlFor="edit-proj-name">Project Name</FieldLabel>
            <Input
              id="edit-proj-name"
              type="text"
              required
              autoFocus
              maxLength={64}
              value={name}
              onChange={(e) => setName(e.target.value)}
            />
          </Field>
          <Field>
            <FieldLabel htmlFor="edit-proj-desc">Description</FieldLabel>
            <Input
              id="edit-proj-desc"
              type="text"
              maxLength={240}
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder="Optional summary"
            />
          </Field>
          <DialogFooter>
            <Button type="button" variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit" variant="primary" loading={saving} disabled={!name.trim()}>
              Save Changes
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}

interface DeleteProjectDialogProps {
  serverId: string
  project: Project | null
  allProjects: Project[]
  databases: Database[]
  onClose: () => void
  onDeleted: (projectId: string) => void
}

export function DeleteProjectDialog({
  serverId,
  project,
  allProjects,
  databases,
  onClose,
  onDeleted,
}: DeleteProjectDialogProps) {
  const projectDbs = project ? databases.filter((d) => d.project_id === project.id) : []
  const otherProjects = project ? allProjects.filter((p) => p.id !== project.id) : []
  const canMove = otherProjects.length > 0

  const [dbAction, setDbAction] = useState<'move' | 'delete'>(canMove ? 'move' : 'delete')
  const [targetId, setTargetId] = useState<string>(otherProjects[0]?.id ?? '')
  const [confirmName, setConfirmName] = useState('')
  const [deleting, setDeleting] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (!project) return
    const others = allProjects.filter((p) => p.id !== project.id)
    setDbAction(others.length > 0 ? 'move' : 'delete')
    setTargetId(others[0]?.id ?? '')
    setConfirmName('')
    setError(null)
  }, [project, allProjects])

  const needsTypedConfirm =
    Boolean(project) && (project!.app_count > 0 || (projectDbs.length > 0 && dbAction === 'delete'))
  const confirmed = !needsTypedConfirm || confirmName.trim() === project?.name

  async function handleDelete(e: FormEvent) {
    e.preventDefault()
    if (!project || !confirmed) return
    setDeleting(true)
    setError(null)
    try {
      await desktopApi.deleteProject(serverId, project.id, {
        databasesAction: projectDbs.length > 0 ? dbAction : 'delete',
        targetProjectId: projectDbs.length > 0 && dbAction === 'move' ? targetId : null,
      })
      toast.success(`Deleted project "${project.name}"`)
      onDeleted(project.id)
      onClose()
    } catch (err) {
      setError(String(err))
    } finally {
      setDeleting(false)
    }
  }

  return (
    <Dialog open={Boolean(project)} onOpenChange={(open) => !open && onClose()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2 text-fail">
            <Trash2 className="size-4" />
            Delete Project
          </DialogTitle>
          <DialogDescription>
            {project && (
              <>
                Permanently remove <strong className="text-foreground">{project.name}</strong>
                {project.app_count > 0
                  ? ` and its ${project.app_count} ${project.app_count === 1 ? 'application' : 'applications'} (containers, domains, and env vars).`
                  : '.'}
              </>
            )}
          </DialogDescription>
        </DialogHeader>

        {project && (
          <form onSubmit={handleDelete} className="space-y-4">
            {error && (
              <Banner role="alert" status="error" contrast="high" size="compact">
                <BannerDescription>{error}</BannerDescription>
              </Banner>
            )}

            {projectDbs.length > 0 && (
              <div className="space-y-2.5 rounded-xl border border-border/80 bg-surface p-3.5">
                <div className="text-[13px] font-medium text-foreground">
                  This project owns {projectDbs.length}{' '}
                  {projectDbs.length === 1 ? 'database' : 'databases'}
                </div>
                <div className="space-y-2">
                  {canMove && (
                    <label
                      className={cn(
                        'flex cursor-pointer items-start gap-2.5 rounded-lg border p-2.5 text-[13px] transition-colors',
                        dbAction === 'move'
                          ? 'border-primary bg-primary/8'
                          : 'border-border/80 bg-card hover:bg-hover',
                      )}
                    >
                      <input
                        type="radio"
                        name="db-fate"
                        checked={dbAction === 'move'}
                        onChange={() => setDbAction('move')}
                        className="mt-0.5 accent-[var(--primary)]"
                      />
                      <div className="min-w-0 flex-1 space-y-2">
                        <div className="font-medium text-foreground">
                          Move databases to another project
                        </div>
                        <div className="text-[12px] text-muted-foreground">
                          Databases stay running without interruption; only their project label
                          changes.
                        </div>
                        {dbAction === 'move' && (
                          <Select value={targetId} onValueChange={setTargetId}>
                            <SelectTrigger className="w-full" placeholder="Target project" />
                            <SelectContent>
                              {otherProjects.map((p, idx) => (
                                <SelectItem key={p.id} index={idx} value={p.id}>
                                  {p.name}
                                </SelectItem>
                              ))}
                            </SelectContent>
                          </Select>
                        )}
                      </div>
                    </label>
                  )}

                  <label
                    className={cn(
                      'flex cursor-pointer items-start gap-2.5 rounded-lg border p-2.5 text-[13px] transition-colors',
                      dbAction === 'delete'
                        ? 'border-fail bg-fail/8'
                        : 'border-border/80 bg-card hover:bg-hover',
                    )}
                  >
                    <input
                      type="radio"
                      name="db-fate"
                      checked={dbAction === 'delete'}
                      onChange={() => setDbAction('delete')}
                      className="mt-0.5 accent-[var(--fail)]"
                    />
                    <div className="min-w-0 flex-1">
                      <div className="font-medium text-fail">
                        Delete databases and all their backups
                      </div>
                      <div className="text-[12px] text-muted-foreground">
                        Permanently destroys {projectDbs.map((d) => d.name).join(', ')} and their
                        volumes.
                      </div>
                    </div>
                  </label>
                </div>
              </div>
            )}

            {needsTypedConfirm && (
              <Field>
                <FieldLabel htmlFor="delete-proj-confirm">
                  Type <code className="font-mono text-foreground">{project.name}</code> to confirm
                </FieldLabel>
                <Input
                  id="delete-proj-confirm"
                  type="text"
                  value={confirmName}
                  onChange={(e) => setConfirmName(e.target.value)}
                  placeholder={project.name}
                  className="font-mono"
                />
              </Field>
            )}

            <DialogFooter>
              <Button type="button" variant="ghost" onClick={onClose}>
                Cancel
              </Button>
              <Button
                type="submit"
                variant="destructive"
                leadingIcon={Trash2}
                loading={deleting}
                disabled={!confirmed || (projectDbs.length > 0 && dbAction === 'move' && !targetId)}
              >
                Delete Project
              </Button>
            </DialogFooter>
          </form>
        )}
      </DialogContent>
    </Dialog>
  )
}
