import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { Eye, EyeOff, KeyRound, Pencil, Plus, Trash2 } from 'lucide-react'
import { useState, type FormEvent } from 'react'

import { api, unwrap, type Schemas } from '@/api/client'
import { envQuery, meQuery } from '@/api/queries'
import { CopyButton } from '@/components/app/copy-button'
import { Panel } from '@/components/app/page'
import { EmptyState, ErrorState } from '@/components/app/state'
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
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Field, FieldContent, FieldDescription, FieldGroup, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { Switch } from '@/components/ui/switch'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { Textarea } from '@/components/ui/textarea'
import { hasRole } from '@/lib/nav'

type EnvVar = Schemas['EnvVar']

/** `'new'` opens the dialog empty; a variable opens it for editing. */
type DialogTarget = EnvVar | 'new' | null

const MASK = '••••••••••••'

export const Route = createFileRoute('/_app/apps/$appId/environment')({
  staticData: { crumb: 'Environment' },
  component: AppEnvironment,
})

function AppEnvironment() {
  const { appId } = Route.useParams()
  const vars = useQuery(envQuery(appId))
  const { data: me } = useQuery(meQuery)
  const canEdit = me ? hasRole(me.role, 'developer') : false

  const [target, setTarget] = useState<DialogTarget>(null)
  const [deleting, setDeleting] = useState<EnvVar | null>(null)
  // Secret values the person chose to reveal. Held only in this component.
  const [revealed, setRevealed] = useState<Record<string, string>>({})

  const forget = (key: string) =>
    setRevealed((prev) => {
      const { [key]: _removed, ...rest } = prev
      return rest
    })

  const reveal = useMutation({
    mutationFn: (key: string) =>
      unwrap(api.POST('/api/v1/apps/{id}/env/{key}/reveal', { params: { path: { id: appId, key } } })),
    onSuccess: (data) => setRevealed((prev) => ({ ...prev, [data.key]: data.value })),
  })
  const revealing = reveal.isPending ? reveal.variables : undefined

  const addButton = canEdit ? (
    <Button onClick={() => setTarget('new')}>
      <Plus data-icon="inline-start" />
      Add variable
    </Button>
  ) : undefined

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center justify-between gap-x-6 gap-y-3">
        <p className="text-muted-foreground">Changes apply on the next deploy.</p>
        {vars.data?.length ? addButton : null}
      </div>

      {vars.isPending ? (
        <Panel>
          <div className="space-y-2">
            {Array.from({ length: 3 }, (_, i) => (
              <Skeleton key={i} className="h-8" />
            ))}
          </div>
        </Panel>
      ) : vars.isError ? (
        <ErrorState error={vars.error} onRetry={() => void vars.refetch()} />
      ) : vars.data.length === 0 ? (
        <EmptyState icon={KeyRound} title="No variables yet" action={addButton}>
          Variables are passed to the container as environment variables. Mark API keys and passwords as secret.
        </EmptyState>
      ) : (
        <Panel flush>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="pl-4">Key</TableHead>
                <TableHead>Value</TableHead>
                <TableHead className="w-28 pr-4" />
              </TableRow>
            </TableHeader>
            <TableBody>
              {vars.data.map((v) => {
                const shown: string | undefined = v.is_secret ? revealed[v.key] : (v.value ?? '')
                const isRevealed = v.is_secret && revealed[v.key] !== undefined
                return (
                  <TableRow key={v.key}>
                    <TableCell className="pl-4">
                      <code className="block max-w-64 truncate text-[0.8125rem] font-medium" title={v.key}>
                        {v.key}
                      </code>
                    </TableCell>
                    <TableCell>
                      <code
                        className="block max-w-xs truncate text-[0.8125rem] text-muted-foreground sm:max-w-md"
                        title={shown}
                      >
                        {shown ?? MASK}
                      </code>
                    </TableCell>
                    <TableCell className="pr-4">
                      <div className="flex items-center justify-end gap-0.5">
                        {v.is_secret && canEdit ? (
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            aria-label={isRevealed ? `Hide value of ${v.key}` : `Show value of ${v.key}`}
                            disabled={revealing === v.key}
                            onClick={() => (isRevealed ? forget(v.key) : reveal.mutate(v.key))}
                          >
                            {revealing === v.key ? <Spinner /> : isRevealed ? <EyeOff /> : <Eye />}
                          </Button>
                        ) : null}
                        {shown !== undefined ? <CopyButton value={shown} label={`Copy value of ${v.key}`} /> : null}
                        {canEdit ? (
                          <>
                            <Button
                              variant="ghost"
                              size="icon-sm"
                              aria-label={`Edit ${v.key}`}
                              onClick={() => setTarget(v)}
                            >
                              <Pencil />
                            </Button>
                            <Button
                              variant="ghost"
                              size="icon-sm"
                              aria-label={`Delete ${v.key}`}
                              onClick={() => setDeleting(v)}
                            >
                              <Trash2 />
                            </Button>
                          </>
                        ) : null}
                      </div>
                    </TableCell>
                  </TableRow>
                )
              })}
            </TableBody>
          </Table>
        </Panel>
      )}

      <VariableDialog appId={appId} target={target} onClose={() => setTarget(null)} onSaved={forget} />
      <DeleteVariableDialog appId={appId} variable={deleting} onClose={() => setDeleting(null)} onDeleted={forget} />
    </div>
  )
}

function VariableDialog({
  appId,
  target,
  onClose,
  onSaved,
}: {
  appId: string
  target: DialogTarget
  onClose: () => void
  onSaved: (key: string) => void
}) {
  return (
    <Dialog open={target !== null} onOpenChange={(open) => (open ? undefined : onClose())}>
      <DialogContent>
        {/* Keyed so each variable starts from its own values. */}
        {target ? (
          <VariableForm
            key={target === 'new' ? 'new' : target.key}
            appId={appId}
            existing={target === 'new' ? null : target}
            onClose={onClose}
            onSaved={onSaved}
          />
        ) : null}
      </DialogContent>
    </Dialog>
  )
}

function VariableForm({
  appId,
  existing,
  onClose,
  onSaved,
}: {
  appId: string
  existing: EnvVar | null
  onClose: () => void
  onSaved: (key: string) => void
}) {
  const queryClient = useQueryClient()
  const [secret, setSecret] = useState(existing?.is_secret ?? true)
  const editing = existing !== null

  const save = useMutation({
    mutationFn: (body: Schemas['SetEnvVarRequest']) =>
      unwrap(api.PUT('/api/v1/apps/{id}/env', { params: { path: { id: appId } }, body })),
    meta: { success: 'Variable saved' },
    onSuccess: async (_data, body) => {
      onSaved(body.key)
      onClose()
      await queryClient.invalidateQueries({ queryKey: ['apps', appId] })
    },
  })

  function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault()
    const form = new FormData(e.currentTarget)
    save.mutate({
      key: editing ? existing.key : String(form.get('key') ?? '').trim(),
      value: String(form.get('value') ?? ''),
      is_secret: secret,
    })
  }

  return (
    <form onSubmit={submit} className="contents">
      <DialogHeader>
        <DialogTitle>{editing ? `Edit ${existing.key}` : 'Add variable'}</DialogTitle>
        <DialogDescription>
          {editing && existing.is_secret
            ? 'Secret values are never shown here. Enter a new value to replace the current one.'
            : 'The container reads this when it starts. Redeploy the app to apply it.'}
        </DialogDescription>
      </DialogHeader>
      <FieldGroup>
        <Field>
          <FieldLabel htmlFor="env-key">Key</FieldLabel>
          <Input
            id="env-key"
            name="key"
            className="font-mono"
            required
            maxLength={128}
            pattern="[A-Za-z_][A-Za-z0-9_]*"
            title="Letters, digits and underscores, not starting with a digit"
            placeholder="DATABASE_URL"
            defaultValue={existing?.key}
            readOnly={editing}
            autoFocus={!editing}
            autoCapitalize="none"
            autoComplete="off"
            spellCheck={false}
          />
          <FieldDescription>Letters, digits and underscores. Cannot start with a digit.</FieldDescription>
        </Field>
        <Field>
          <FieldLabel htmlFor="env-value">Value</FieldLabel>
          <Textarea
            id="env-value"
            name="value"
            className="font-mono"
            rows={3}
            required={secret}
            defaultValue={existing && !existing.is_secret ? (existing.value ?? '') : ''}
            placeholder={editing && existing.is_secret ? 'Enter a new value' : undefined}
            autoFocus={editing}
            autoCapitalize="none"
            autoComplete="off"
            spellCheck={false}
          />
        </Field>
        <Field orientation="horizontal">
          <FieldContent>
            <FieldLabel htmlFor="env-secret">Secret</FieldLabel>
            <FieldDescription>Hidden in the dashboard and encrypted on disk</FieldDescription>
          </FieldContent>
          <Switch id="env-secret" checked={secret} onCheckedChange={setSecret} />
        </Field>
      </FieldGroup>
      <DialogFooter>
        <Button type="button" variant="outline" onClick={onClose}>
          Cancel
        </Button>
        <Button type="submit" disabled={save.isPending}>
          {save.isPending ? <Spinner data-icon="inline-start" /> : null}
          {editing ? 'Save variable' : 'Add variable'}
        </Button>
      </DialogFooter>
    </form>
  )
}

function DeleteVariableDialog({
  appId,
  variable,
  onClose,
  onDeleted,
}: {
  appId: string
  variable: EnvVar | null
  onClose: () => void
  onDeleted: (key: string) => void
}) {
  const queryClient = useQueryClient()
  const remove = useMutation({
    mutationFn: (key: string) =>
      unwrap(api.DELETE('/api/v1/apps/{id}/env/{key}', { params: { path: { id: appId, key } } })),
    meta: { success: 'Variable deleted' },
    onSuccess: async (_data, key) => {
      onDeleted(key)
      onClose()
      await queryClient.invalidateQueries({ queryKey: ['apps', appId] })
    },
  })

  return (
    <AlertDialog open={variable !== null} onOpenChange={(open) => (open ? undefined : onClose())}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Delete {variable?.key}?</AlertDialogTitle>
          <AlertDialogDescription>
            The container stops receiving this variable on the next deploy. The value cannot be recovered afterwards.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Keep variable</AlertDialogCancel>
          <AlertDialogAction
            variant="destructive"
            disabled={remove.isPending}
            onClick={() => variable && remove.mutate(variable.key)}
          >
            Delete variable
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
