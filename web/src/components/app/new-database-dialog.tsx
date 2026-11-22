import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useState, type FormEvent } from 'react'
import { api, unwrap, type Schemas } from '@/api/client'
import { databaseEnginesQuery, databasesQuery, projectsQuery } from '@/api/queries'
import { EngineMark } from '@/components/app/service-icons'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/fluid/dialog'
import { Field, FieldDescription, FieldGroup, FieldLabel } from '@/components/fluid/field'
import { Input } from '@/components/fluid/input'
import { Select, SelectContent, SelectItem, SelectTrigger } from '@/components/fluid/select'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { cn } from '@/lib/utils'

type Engine = Schemas['DatabaseEngine']

/**
 * Creates a database. Opened from a project, the project is fixed and named
 * in the description; opened from the Databases page, it is a choice.
 */
export function NewDatabaseDialog({
  open,
  onOpenChange,
  projectId: fixedProject,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  projectId?: string
}) {
  const queryClient = useQueryClient()
  const { data: projects = [] } = useQuery(projectsQuery)
  const engines = useQuery(databaseEnginesQuery)
  const [projectId, setProjectId] = useState<string>()
  const [engine, setEngine] = useState<Engine>('postgres')
  const [version, setVersion] = useState<string>()

  const project = fixedProject ?? projectId ?? projects[0]?.id
  const projectName = projects.find((p) => p.id === project)?.name
  const versions = engines.data?.find((e) => e.engine === engine)?.versions ?? []
  const effectiveVersion = version && versions.includes(version) ? version : versions[0]

  const create = useMutation({
    mutationFn: (body: Schemas['CreateDatabaseRequest']) => unwrap(api.POST('/api/v1/databases', { body })),
    meta: { success: 'Database starting' },
    onSuccess: async () => {
      onOpenChange(false)
      await queryClient.invalidateQueries({ queryKey: databasesQuery.queryKey })
    },
  })

  function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault()
    if (!project) return
    const name = String(new FormData(e.currentTarget).get('name') ?? '').trim()
    create.mutate({ project_id: project, name, engine, version: effectiveVersion ?? null })
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent size="lg">
        <form onSubmit={submit} className="contents">
          <DialogHeader>
            <DialogTitle>New database</DialogTitle>
            <DialogDescription>
              {fixedProject && projectName ? `In ${projectName}. ` : ''}Credentials are generated automatically.
            </DialogDescription>
          </DialogHeader>
          <FieldGroup>
            <Field>
              <FieldLabel id="db-engine-label">Engine</FieldLabel>
              {engines.isPending ? (
                <div className="grid grid-cols-3 gap-2 sm:grid-cols-5">
                  {Array.from({ length: 5 }, (_, i) => (
                    <Skeleton key={i} className="h-[72px]" />
                  ))}
                </div>
              ) : (
                <div role="radiogroup" aria-labelledby="db-engine-label" className="grid grid-cols-3 gap-2 sm:grid-cols-5">
                  {(engines.data ?? []).map((e) => {
                    const selected = e.engine === engine
                    return (
                      <button
                        key={e.engine}
                        type="button"
                        role="radio"
                        aria-checked={selected}
                        onClick={() => setEngine(e.engine)}
                        className={cn(
                          'flex flex-col items-center gap-2 rounded-lg px-2 py-3 text-[13px] outline-none transition-[box-shadow,background-color] duration-150',
                          'shadow-[0_0_0_1px_var(--border)] hover:bg-hover focus-visible:ring-1 focus-visible:ring-[color:var(--focus-ring)]',
                          selected && 'bg-tint shadow-[0_0_0_1.5px_var(--foreground)]',
                        )}
                      >
                        <EngineMark engine={e.engine} className="size-6" />
                        <span className={cn('font-medium', !selected && 'text-muted-foreground')}>{e.label}</span>
                      </button>
                    )
                  })}
                </div>
              )}
            </Field>
            <div className="grid gap-4 sm:grid-cols-[1fr_9rem]">
              <Field>
                <FieldLabel htmlFor="db-name">Name</FieldLabel>
                <Input id="db-name" name="name" required maxLength={64} placeholder={`${engine}-main`} autoFocus autoComplete="off" spellCheck={false} />
                <FieldDescription>Also used as the internal hostname.</FieldDescription>
              </Field>
              <Field>
                <FieldLabel htmlFor="db-version">Version</FieldLabel>
                <Select value={effectiveVersion ?? ''} onValueChange={(v) => setVersion(v)}>
                  <SelectTrigger id="db-version" className="w-full" />
                  <SelectContent>
                    {versions.map((v, i) => (
                      <SelectItem key={v} index={i} value={v}>
                        {v}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </Field>
            </div>
            {fixedProject ? null : (
              <Field>
                <FieldLabel htmlFor="db-project">Project</FieldLabel>
                {projects.length === 0 ? (
                  <FieldDescription>Create a project first; databases belong to one.</FieldDescription>
                ) : (
                  <Select value={project ?? ''} onValueChange={(v) => setProjectId(v)}>
                    <SelectTrigger id="db-project" className="w-full" />
                    <SelectContent>
                      {projects.map((p, i) => (
                        <SelectItem key={p.id} index={i} value={p.id}>
                          {p.name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                )}
              </Field>
            )}
          </FieldGroup>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" loading={create.isPending} disabled={create.isPending || !project || !effectiveVersion}>
              Create database
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
