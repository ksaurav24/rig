import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { KeyRound, Trash2, UserPlus } from 'lucide-react'
import { useState, type FormEvent } from 'react'
import { api, unwrap, type Schemas } from '@/api/client'
import { membersQuery, meQuery } from '@/api/queries'
import { CopyButton } from '@/components/app/copy-button'
import { Page, Panel } from '@/components/app/page'
import { ErrorState } from '@/components/app/state'
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
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Field, FieldDescription, FieldGroup, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { timeAgo } from '@/lib/format'
import { hasRole, ROLES } from '@/lib/nav'

type Role = Schemas['Role']
type Member = Schemas['Member']
type Handover = Schemas['MemberWithPassword']
type Confirm = { kind: 'remove' | 'reset'; member: Member }

export const Route = createFileRoute('/_app/settings/team')({
  staticData: { crumb: 'Team' },
  loader: ({ context }) => void context.queryClient.prefetchQuery(membersQuery),
  component: TeamPage,
})

function TeamPage() {
  const members = useQuery(membersQuery)
  const me = useQuery(meQuery).data
  const [adding, setAdding] = useState(false)
  const [handover, setHandover] = useState<Handover | null>(null)
  const [confirm, setConfirm] = useState<Confirm | null>(null)

  // Mirrors the server's rule; the server still enforces it.
  const canManage = (m: Member) =>
    !!me && m.user_id !== me.user.id && (me.role === 'owner' || !hasRole(m.role, 'admin'))

  return (
    <Page
      title="Team"
      description="People who can sign in to this Rig. Rig sends no email: adding someone gives you a temporary password to pass on."
      actions={
        <Button onClick={() => setAdding(true)}>
          <UserPlus data-icon="inline-start" />
          Add member
        </Button>
      }
    >
      {members.isPending ? (
        <Panel>
          <div className="space-y-2">
            {Array.from({ length: 3 }, (_, i) => (
              <Skeleton key={i} className="h-9" />
            ))}
          </div>
        </Panel>
      ) : members.isError ? (
        <ErrorState error={members.error} onRetry={() => void members.refetch()} />
      ) : (
        <Panel flush>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="pl-4">Member</TableHead>
                <TableHead className="w-40">Role</TableHead>
                <TableHead className="hidden md:table-cell">Last sign-in</TableHead>
                <TableHead className="w-20 pr-4" />
              </TableRow>
            </TableHeader>
            <TableBody>
              {members.data.map((m) => (
                <TableRow key={m.user_id}>
                  <TableCell className="pl-4">
                    <div className="flex items-center gap-2">
                      <span className="truncate font-medium">{m.name}</span>
                      {m.user_id === me?.user.id ? <Badge variant="outline">You</Badge> : null}
                    </div>
                    <div className="truncate text-[0.8125rem] text-muted-foreground">{m.email}</div>
                  </TableCell>
                  <TableCell>
                    {canManage(m) && me ? (
                      <RoleSelect member={m} myRole={me.role} />
                    ) : (
                      <Badge variant="secondary" className="capitalize">
                        {m.role}
                      </Badge>
                    )}
                  </TableCell>
                  <TableCell className="hidden text-muted-foreground md:table-cell">
                    {m.last_login_at ? timeAgo(m.last_login_at) : 'Never'}
                  </TableCell>
                  <TableCell className="pr-4 text-right whitespace-nowrap">
                    {canManage(m) ? (
                      <>
                        <Tooltip>
                          <TooltipTrigger
                            render={
                              <Button
                                variant="ghost"
                                size="icon-sm"
                                aria-label={`Reset password for ${m.name}`}
                                onClick={() => setConfirm({ kind: 'reset', member: m })}
                              />
                            }
                          >
                            <KeyRound />
                          </TooltipTrigger>
                          <TooltipContent>Reset password</TooltipContent>
                        </Tooltip>
                        <Tooltip>
                          <TooltipTrigger
                            render={
                              <Button
                                variant="ghost"
                                size="icon-sm"
                                aria-label={`Remove ${m.name}`}
                                onClick={() => setConfirm({ kind: 'remove', member: m })}
                              />
                            }
                          >
                            <Trash2 />
                          </TooltipTrigger>
                          <TooltipContent>Remove member</TooltipContent>
                        </Tooltip>
                      </>
                    ) : null}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </Panel>
      )}

      <AddMemberDialog open={adding} onOpenChange={setAdding} onAdded={setHandover} myRole={me?.role ?? 'viewer'} />
      <ConfirmDialog confirm={confirm} onClose={() => setConfirm(null)} onReset={setHandover} />
      <HandoverDialog handover={handover} onClose={() => setHandover(null)} />
    </Page>
  )
}

function RoleSelect({ member, myRole }: { member: Member; myRole: Role }) {
  const queryClient = useQueryClient()
  const allowed = ROLES.filter((r) => hasRole(myRole, r.value))
  const update = useMutation({
    mutationFn: (role: Role) =>
      unwrap(api.PATCH('/api/v1/members/{user_id}', { params: { path: { user_id: member.user_id } }, body: { role } })),
    meta: { success: `${member.name}'s role updated` },
    onSettled: () => queryClient.invalidateQueries({ queryKey: membersQuery.queryKey }),
  })
  // Show the choice at once; a failure refetches and snaps back.
  const value = update.isPending ? update.variables : member.role

  return (
    <Select items={allowed} value={value} onValueChange={(v) => v !== member.role && update.mutate(v as Role)}>
      <SelectTrigger size="sm" className="w-32" aria-label={`Role for ${member.name}`} disabled={update.isPending}>
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {allowed.map((r) => (
          <SelectItem key={r.value} value={r.value}>
            {r.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}

function AddMemberDialog({
  open,
  onOpenChange,
  onAdded,
  myRole,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  onAdded: (h: Handover) => void
  myRole: Role
}) {
  const queryClient = useQueryClient()
  const allowed = ROLES.filter((r) => hasRole(myRole, r.value))
  const [role, setRole] = useState<Role>('developer')

  const add = useMutation({
    mutationFn: (body: Schemas['AddMemberRequest']) => unwrap(api.POST('/api/v1/members', { body })),
    onSuccess: async (result) => {
      onOpenChange(false)
      onAdded(result)
      await queryClient.invalidateQueries({ queryKey: membersQuery.queryKey })
    },
  })

  function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault()
    const form = new FormData(e.currentTarget)
    add.mutate({ name: String(form.get('name') ?? ''), email: String(form.get('email') ?? ''), role })
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <form onSubmit={submit} className="contents">
          <DialogHeader>
            <DialogTitle>Add member</DialogTitle>
            <DialogDescription>You get a temporary password to pass on. They can change it under Account.</DialogDescription>
          </DialogHeader>
          <FieldGroup>
            <Field>
              <FieldLabel htmlFor="member-name">Name</FieldLabel>
              <Input id="member-name" name="name" required maxLength={80} autoFocus autoComplete="off" />
            </Field>
            <Field>
              <FieldLabel htmlFor="member-email">Email</FieldLabel>
              <Input id="member-email" name="email" type="email" required maxLength={254} autoComplete="off" />
              <FieldDescription>Used to sign in. Rig never sends mail to it.</FieldDescription>
            </Field>
            <Field>
              <FieldLabel htmlFor="member-role">Role</FieldLabel>
              <Select items={allowed} value={role} onValueChange={(v) => setRole(v as Role)}>
                <SelectTrigger id="member-role" className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {allowed.map((r) => (
                    <SelectItem key={r.value} value={r.value}>
                      {r.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <FieldDescription>{ROLES.find((r) => r.value === role)?.hint}.</FieldDescription>
            </Field>
          </FieldGroup>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={add.isPending}>
              {add.isPending ? <Spinner data-icon="inline-start" /> : null}
              Add member
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}

function HandoverDialog({ handover, onClose }: { handover: Handover | null; onClose: () => void }) {
  return (
    <Dialog open={handover !== null} onOpenChange={(open) => (open ? undefined : onClose())}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Pass on this password</DialogTitle>
          <DialogDescription>
            This is the one time it is shown. {handover?.member.name} signs in at {window.location.origin} with{' '}
            {handover?.member.email}.
          </DialogDescription>
        </DialogHeader>
        <div className="flex items-center gap-1 rounded-md border bg-muted/50 py-1 pr-1 pl-3">
          <code className="min-w-0 flex-1 overflow-x-auto py-1 text-[0.8125rem] whitespace-nowrap">
            {handover?.temporary_password}
          </code>
          <CopyButton value={handover?.temporary_password ?? ''} label="Copy password" />
        </div>
        <DialogFooter>
          <Button onClick={onClose}>I have copied it</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function ConfirmDialog({
  confirm,
  onClose,
  onReset,
}: {
  confirm: Confirm | null
  onClose: () => void
  onReset: (h: Handover) => void
}) {
  const queryClient = useQueryClient()
  const done = async () => {
    onClose()
    await queryClient.invalidateQueries({ queryKey: membersQuery.queryKey })
  }
  const remove = useMutation({
    mutationFn: (user_id: string) => unwrap(api.DELETE('/api/v1/members/{user_id}', { params: { path: { user_id } } })),
    meta: { success: 'Member removed' },
    onSuccess: done,
  })
  const reset = useMutation({
    mutationFn: (user_id: string) =>
      unwrap(api.POST('/api/v1/members/{user_id}/reset-password', { params: { path: { user_id } } })),
    onSuccess: async (result) => {
      onReset(result)
      await done()
    },
  })
  const removing = confirm?.kind === 'remove'
  const name = confirm?.member.name

  return (
    <AlertDialog open={confirm !== null} onOpenChange={(open) => (open ? undefined : onClose())}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{removing ? `Remove ${name}?` : `Reset ${name}'s password?`}</AlertDialogTitle>
          <AlertDialogDescription>
            {removing
              ? 'They are signed out and their access tokens stop working immediately. Anything they deployed stays.'
              : 'Their current password stops working and they are signed out everywhere. You get a new temporary password to pass on.'}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction
            variant={removing ? 'destructive' : 'default'}
            disabled={remove.isPending || reset.isPending}
            onClick={() => confirm && (removing ? remove : reset).mutate(confirm.member.user_id)}
          >
            {removing ? 'Remove member' : 'Reset password'}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
