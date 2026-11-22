import { useMutation, useQueryClient } from '@tanstack/react-query'
import { useNavigate, useRouter } from '@tanstack/react-router'
import { useState, type FormEvent, type ReactNode } from 'react'
import { api, unwrap, type Schemas } from '@/api/client'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/fluid/dialog'
import { Field, FieldGroup, FieldLabel } from '@/components/fluid/field'
import { Input } from '@/components/fluid/input'
import { Select, SelectContent, SelectItem, SelectTrigger } from '@/components/fluid/select'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'

type Project = Schemas['Project']
type Database = Schemas['Database']

/** Creates a project and lands on it, where its first service is added. */
export function NewProjectDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (open: boolean) => void }) {
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const create = useMutation({
    mutationFn: (body: Schemas['CreateProjectRequest']) => unwrap(api.POST('/api/v1/projects', { body })),
    meta: { success: 'Project created' },
    onSuccess: async (project) => {
      onOpenChange(false)
      await queryClient.invalidateQueries({ queryKey: ['projects'] })
      await navigate({ to: '/projects/$projectId', params: { projectId: project.id } })
    },
  })

  function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault()
    const form = new FormData(e.currentTarget)
    create.mutate({
      name: String(form.get('name') ?? '').trim(),
      description: String(form.get('description') ?? '').trim(),
    })
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent size="sm">
        <form onSubmit={submit} className="contents">
          <DialogHeader>
            <DialogTitle>New project</DialogTitle>
            <DialogDescription>Services inside a project share a private network.</DialogDescription>
          </DialogHeader>
          <FieldGroup>
            <Field>
              <FieldLabel htmlFor="project-name">Name</FieldLabel>
              <Input id="project-name" name="name" required maxLength={64} placeholder="Storefront" autoFocus autoComplete="off" />
            </Field>
            <Field>
              <FieldLabel htmlFor="project-description">Description (optional)</FieldLabel>
              <Input id="project-description" name="description" maxLength={200} autoComplete="off" />
            </Field>
          </FieldGroup>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" loading={create.isPending} disabled={create.isPending}>
              Create project
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}

/** Renames a project or changes its description. The slug stays, so nothing addressing it breaks. */
export function EditProjectDialog({ project, onClose }: { project: Project | null; onClose: () => void }) {
  const queryClient = useQueryClient()
  const router = useRouter()
  const update = useMutation({
    mutationFn: ({ id, body }: { id: string; body: Schemas['UpdateProjectRequest'] }) =>
      unwrap(api.PATCH('/api/v1/projects/{id}', { params: { path: { id } }, body })),
    meta: { success: 'Project updated' },
    onSuccess: async () => {
      onClose()
      await Promise.all(['projects', 'apps', 'databases'].map((key) => queryClient.invalidateQueries({ queryKey: [key] })))
      await router.invalidate()
    },
  })

  function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault()
    if (!project) return
    const form = new FormData(e.currentTarget)
    update.mutate({
      id: project.id,
      body: {
        name: String(form.get('name') ?? '').trim(),
        description: String(form.get('description') ?? '').trim(),
      },
    })
  }

  return (
    <Dialog open={project !== null} onOpenChange={(open) => (open ? undefined : onClose())}>
      <DialogContent size="sm">
        <form key={project?.id} onSubmit={submit} className="contents">
          <DialogHeader>
            <DialogTitle>Project settings</DialogTitle>
            <DialogDescription>Renaming does not affect domains or tokens.</DialogDescription>
          </DialogHeader>
          <FieldGroup>
            <Field>
              <FieldLabel htmlFor="edit-project-name">Name</FieldLabel>
              <Input
                id="edit-project-name"
                name="name"
                required
                maxLength={64}
                defaultValue={project?.name}
                autoFocus
                autoComplete="off"
              />
            </Field>
            <Field>
              <FieldLabel htmlFor="edit-project-description">Description (optional)</FieldLabel>
              <Input
                id="edit-project-description"
                name="description"
                maxLength={200}
                defaultValue={project?.description}
                autoComplete="off"
              />
            </Field>
          </FieldGroup>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit" loading={update.isPending} disabled={update.isPending}>
              Save
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}

/**
 * Deletes a project and its apps. Databases are kept by default: they move,
 * data and backups included, to another project. Deleting them instead takes
 * typing the project's name.
 */
export function DeleteProjectDialog({
  project,
  databases,
  projects,
  onClose,
  onDeleted,
}: {
  project: Project | null
  databases: Database[]
  projects: Project[]
  onClose: () => void
  onDeleted?: () => void
}) {
  return (
    <AlertDialog open={project !== null} onOpenChange={(open) => (open ? undefined : onClose())}>
      <AlertDialogContent>
        {project ? (
          <DeleteProjectBody
            key={project.id}
            project={project}
            databases={databases}
            others={projects.filter((p) => p.id !== project.id)}
            onClose={onClose}
            onDeleted={onDeleted}
          />
        ) : null}
      </AlertDialogContent>
    </AlertDialog>
  )
}

function DeleteProjectBody({
  project,
  databases,
  others,
  onClose,
  onDeleted,
}: {
  project: Project
  databases: Database[]
  others: Project[]
  onClose: () => void
  onDeleted?: () => void
}) {
  const queryClient = useQueryClient()
  const [fate, setFate] = useState<'move' | 'delete'>(others.length ? 'move' : 'delete')
  const [target, setTarget] = useState(others[0]?.id ?? '')
  const [typed, setTyped] = useState('')
  const remove = useMutation({
    mutationFn: () =>
      unwrap(
        api.DELETE('/api/v1/projects/{id}', {
          params: {
            path: { id: project.id },
            query: databases.length ? { databases: fate, target: fate === 'move' ? target : undefined } : {},
          },
        }),
      ),
    meta: { success: 'Project deleted' },
    onSuccess: async () => {
      onClose()
      onDeleted?.()
      await Promise.all(['projects', 'apps', 'databases'].map((key) => queryClient.invalidateQueries({ queryKey: [key] })))
    },
  })

  const apps = project.app_count
  const dbs = databases.length
  const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? '' : 's'}`
  const destroysData = dbs > 0 && fate === 'delete'
  const ready = !remove.isPending && (destroysData ? typed.trim() === project.name : fate === 'delete' || dbs === 0 || target !== '')
  const targetName = others.find((p) => p.id === target)?.name

  return (
    <>
      <AlertDialogHeader>
        <AlertDialogTitle>Delete “{project.name}”?</AlertDialogTitle>
        <AlertDialogDescription>
          {apps
            ? `Its ${plural(apps, 'app')} will stop and be deleted along with deployments, variables, domains and volumes.`
            : dbs
              ? 'This project has no apps.'
              : 'This project is empty.'}{' '}
          This cannot be undone.
        </AlertDialogDescription>
      </AlertDialogHeader>

      {dbs ? (
        <fieldset className="grid gap-2">
          <legend className="mb-2 text-[13px] font-medium">
            {plural(dbs, 'database')}: {databases.map((d) => d.name).join(', ')}
          </legend>
          <FateOption
            checked={fate === 'move'}
            disabled={others.length === 0}
            onSelect={() => setFate('move')}
            title="Move to another project"
            hint={
              others.length
                ? 'Data and backups move along. Running databases restart on the new network.'
                : 'No other project exists. Create one first to move them.'
            }
          >
            {fate === 'move' && others.length ? (
              <div className="mt-2">
                <Select value={target} onValueChange={(v) => setTarget(v)}>
                  <SelectTrigger className="w-full" aria-label="Move databases to" />
                  <SelectContent>
                    {others.map((p, i) => (
                      <SelectItem key={p.id} index={i} value={p.id}>
                        {p.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            ) : null}
          </FateOption>
          <FateOption
            checked={fate === 'delete'}
            onSelect={() => setFate('delete')}
            title="Delete them too"
            hint="All data and backups are destroyed."
            danger
          >
            {fate === 'delete' ? (
              <Field className="mt-2">
                <FieldLabel htmlFor="confirm-project-name" className="text-[12.5px] font-normal text-muted-foreground">
                  Type <span className="font-medium text-foreground">{project.name}</span> to confirm
                </FieldLabel>
                <Input
                  id="confirm-project-name"
                  value={typed}
                  onChange={(e) => setTyped(e.target.value)}
                  autoComplete="off"
                  spellCheck={false}
                />
              </Field>
            ) : null}
          </FateOption>
        </fieldset>
      ) : null}

      <AlertDialogFooter>
        <AlertDialogCancel>Keep project</AlertDialogCancel>
        <AlertDialogAction
          variant="destructive"
          disabled={!ready}
          onClick={(e) => {
            e.preventDefault()
            remove.mutate()
          }}
        >
          {dbs && fate === 'move' && targetName ? `Delete and move to ${targetName}` : 'Delete project'}
        </AlertDialogAction>
      </AlertDialogFooter>
    </>
  )
}

function FateOption({
  checked,
  disabled,
  onSelect,
  title,
  hint,
  danger,
  children,
}: {
  checked: boolean
  disabled?: boolean
  onSelect: () => void
  title: string
  hint: string
  danger?: boolean
  children?: ReactNode
}) {
  return (
    <div
      className={cn(
        'rounded-lg px-3 py-2.5 shadow-[0_0_0_1px_var(--border)] transition-shadow',
        checked && (danger ? 'shadow-[0_0_0_1.5px_var(--fail)]' : 'shadow-[0_0_0_1.5px_var(--primary)]'),
        disabled && 'opacity-60',
      )}
    >
      <label className={cn('flex items-start gap-2.5', disabled ? 'cursor-not-allowed' : 'cursor-pointer')}>
        <input
          type="radio"
          name="database-fate"
          checked={checked}
          disabled={disabled}
          onChange={onSelect}
          className={cn('mt-0.5 size-4', danger ? 'accent-[var(--fail)]' : 'accent-[var(--primary)]')}
        />
        <span className="grid gap-0.5 text-[13px]">
          <span className="font-medium">{title}</span>
          <span className="leading-snug text-muted-foreground">{hint}</span>
        </span>
      </label>
      {children ? <div className="pl-[26px]">{children}</div> : null}
    </div>
  )
}
