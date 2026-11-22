import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Archive, Download, RotateCcw, Trash2 } from 'lucide-react'
import { useState } from 'react'
import { api, unwrap, type Schemas } from '@/api/client'
import { backupsQuery } from '@/api/queries'
import { EmptyState, ErrorState, Status } from '@/components/app/state'
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
import { Button, buttonVariants } from '@/components/ui/button'
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from '@/components/ui/sheet'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { bytes, fullDate, timeAgo } from '@/lib/format'
import { BACKUP_STATUS } from '@/lib/status'

type Database = Schemas['Database']
type Backup = Schemas['Backup']
type Pending = { kind: 'restore' | 'delete'; backup: Backup }

/** Side panel listing a database's backups, with take / download / restore / delete. */
export function BackupsSheet({
  database,
  canBackup,
  canManage,
  onClose,
}: {
  database: Database | null
  /** Developer and up may take backups. */
  canBackup: boolean
  /** Admins may download, restore and delete them. */
  canManage: boolean
  onClose: () => void
}) {
  return (
    <Sheet open={database !== null} onOpenChange={(open) => (open ? undefined : onClose())}>
      <SheetContent className="gap-0 data-[side=right]:w-full data-[side=right]:sm:max-w-md">
        {database ? <Body database={database} canBackup={canBackup} canManage={canManage} /> : null}
      </SheetContent>
    </Sheet>
  )
}

function Body({ database, canBackup, canManage }: { database: Database; canBackup: boolean; canManage: boolean }) {
  const queryClient = useQueryClient()
  const backups = useQuery(backupsQuery(database.id))
  const [pending, setPending] = useState<Pending | null>(null)
  const refresh = () => queryClient.invalidateQueries({ queryKey: backupsQuery(database.id).queryKey })

  const take = useMutation({
    mutationFn: () =>
      unwrap(api.POST('/api/v1/databases/{id}/backups', { params: { path: { id: database.id } } })),
    meta: { success: 'Backup started' },
    onSettled: refresh,
  })
  const running = backups.data?.some((b) => b.status === 'running') ?? false
  const stopped = database.status !== 'running'

  return (
    <>
      <SheetHeader className="border-b pr-12">
        <SheetTitle>Backups of {database.name}</SheetTitle>
        <SheetDescription>
          Stored on this server under the Rig data directory. Download them to keep a copy somewhere else.
          {database.engine === 'redis' ? ' Redis backups can be downloaded but not yet restored from here.' : null}
        </SheetDescription>
        {canBackup ? (
          <div className="flex items-center gap-3 pt-2">
            <Button disabled={stopped || running || take.isPending} onClick={() => take.mutate()}>
              {running || take.isPending ? <Spinner data-icon="inline-start" /> : <Archive data-icon="inline-start" />}
              {running ? 'Backing up' : 'Back up now'}
            </Button>
            {stopped ? <span className="text-[0.8125rem] text-muted-foreground">Start the database first.</span> : null}
          </div>
        ) : null}
      </SheetHeader>

      <div className="min-h-0 flex-1 overflow-y-auto">
        {backups.isPending ? (
          <div className="flex flex-col gap-3 p-4">
            {[0, 1, 2].map((i) => (
              <Skeleton key={i} className="h-12" />
            ))}
          </div>
        ) : backups.isError ? (
          <div className="p-4">
            <ErrorState error={backups.error} onRetry={() => void backups.refetch()} />
          </div>
        ) : backups.data.length === 0 ? (
          <div className="p-4">
            <EmptyState icon={Archive} title="No backups yet">
              {canBackup ? 'Take one before changes you might want to undo.' : 'An admin or developer can take one.'}
            </EmptyState>
          </div>
        ) : (
          <ul className="divide-y">
            {backups.data.map((b) => (
              <BackupRow key={b.id} backup={b} canManage={canManage} onAsk={setPending} />
            ))}
          </ul>
        )}
      </div>

      <ConfirmDialog database={database} pending={pending} onClose={() => setPending(null)} onDone={refresh} />
    </>
  )
}

function BackupRow({
  backup: b,
  canManage,
  onAsk,
}: {
  backup: Backup
  canManage: boolean
  onAsk: (p: Pending) => void
}) {
  const look = BACKUP_STATUS[b.status]
  return (
    <li className="flex items-center gap-3 px-4 py-3">
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="truncate font-medium" title={fullDate(b.created_at)}>
            {timeAgo(b.created_at)}
          </span>
          {b.size_bytes != null ? (
            <span className="text-[0.8125rem] text-muted-foreground tabular-nums">{bytes(b.size_bytes)}</span>
          ) : null}
        </div>
        {b.status === 'failed' && b.error ? (
          <p className="mt-0.5 line-clamp-2 text-[0.8125rem] text-destructive" title={b.error}>
            {b.error}
          </p>
        ) : (
          <Status tone={look.tone} className="mt-0.5 text-muted-foreground">
            {look.label}
          </Status>
        )}
      </div>
      {canManage ? (
        <div className="flex shrink-0 items-center gap-0.5">
          {b.status === 'ok' ? (
            <>
              <Action label="Download">
                <a
                  href={`/api/v1/backups/${b.id}/download`}
                  download
                  aria-label="Download backup"
                  className={buttonVariants({ variant: 'ghost', size: 'icon-sm' })}
                >
                  <Download />
                </a>
              </Action>
              {b.restorable ? (
                <Action label="Restore">
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label="Restore backup"
                    onClick={() => onAsk({ kind: 'restore', backup: b })}
                  >
                    <RotateCcw />
                  </Button>
                </Action>
              ) : null}
            </>
          ) : null}
          {b.status !== 'running' ? (
            <Action label="Delete">
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label="Delete backup"
                onClick={() => onAsk({ kind: 'delete', backup: b })}
              >
                <Trash2 />
              </Button>
            </Action>
          ) : null}
        </div>
      ) : null}
    </li>
  )
}

function Action({ label, children }: { label: string; children: React.ReactElement }) {
  return (
    <Tooltip>
      <TooltipTrigger render={children} />
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  )
}

function ConfirmDialog({
  database,
  pending,
  onClose,
  onDone,
}: {
  database: Database
  pending: Pending | null
  onClose: () => void
  onDone: () => Promise<void>
}) {
  const restore = useMutation({
    mutationFn: (id: string) => unwrap(api.POST('/api/v1/backups/{id}/restore', { params: { path: { id } } })),
    meta: { success: `${database.name} restored` },
    onSuccess: onClose,
  })
  const remove = useMutation({
    mutationFn: (id: string) => unwrap(api.DELETE('/api/v1/backups/{id}', { params: { path: { id } } })),
    meta: { success: 'Backup deleted' },
    onSuccess: async () => {
      onClose()
      await onDone()
    },
  })
  const busy = restore.isPending || remove.isPending
  const isRestore = pending?.kind === 'restore'
  const when = pending ? timeAgo(pending.backup.created_at) : ''

  return (
    <AlertDialog open={pending !== null} onOpenChange={(open) => (open || busy ? undefined : onClose())}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{isRestore ? `Restore ${database.name}?` : 'Delete this backup?'}</AlertDialogTitle>
          <AlertDialogDescription>
            {isRestore
              ? `Everything in ${database.name} is replaced with the backup from ${when}. Changes made since then are lost. Take a fresh backup first if you might need them.`
              : `The backup from ${when} is erased from the server. This cannot be undone.`}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel disabled={busy}>Cancel</AlertDialogCancel>
          <AlertDialogAction
            variant="destructive"
            disabled={busy}
            onClick={() => {
              if (!pending) return
              if (isRestore) restore.mutate(pending.backup.id)
              else remove.mutate(pending.backup.id)
            }}
          >
            {busy ? <Spinner data-icon="inline-start" /> : null}
            {isRestore ? 'Restore' : 'Delete backup'}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
