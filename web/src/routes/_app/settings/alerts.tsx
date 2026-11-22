import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { Bell, Plus, Send, Trash2 } from 'lucide-react'
import { useMemo, useState, type FormEvent } from 'react'
import { toast } from 'sonner'
import { api, unwrap, type Schemas } from '@/api/client'
import { alertChannelsQuery } from '@/api/queries'
import { CopyButton } from '@/components/app/copy-button'
import { Page, Panel } from '@/components/app/page'
import { EmptyState, ErrorState } from '@/components/app/state'
import { Badge } from '@/components/fluid/badge'
import { CheckboxGroup, CheckboxItem } from '@/components/fluid/checkbox-group'
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
import { Switch } from '@/components/fluid/switch'
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
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { timeAgo } from '@/lib/format'

type Channel = Schemas['AlertChannel']
type AlertEvent = Schemas['AlertEvent']

const EVENTS: { value: AlertEvent; label: string; hint: string }[] = [
  { value: 'deploy_failed', label: 'Deploy failed', hint: 'A deployment did not go live' },
  { value: 'app_crashed', label: 'App crashed', hint: 'The container died and was restarted' },
  { value: 'app_down', label: 'App down', hint: 'The container stopped and stayed stopped' },
  { value: 'backup_failed', label: 'Backup failed', hint: 'A database backup did not finish' },
  { value: 'disk_low', label: 'Disk low', hint: 'Under 5 GB or 10% free on the data disk' },
]
const ALL_EVENTS = EVENTS.map((e) => e.value)

export const Route = createFileRoute('/_app/settings/alerts')({
  staticData: { crumb: 'Alerts' },
  loader: ({ context }) => void context.queryClient.prefetchQuery(alertChannelsQuery),
  component: AlertsPage,
})

function AlertsPage() {
  const channels = useQuery(alertChannelsQuery)
  const [creating, setCreating] = useState(false)
  const [created, setCreated] = useState<Schemas['CreatedAlertChannel'] | null>(null)
  const [removing, setRemoving] = useState<Channel | null>(null)

  const newButton = (
    <Button onClick={() => setCreating(true)}>
      <Plus data-icon="inline-start" />
      New channel
    </Button>
  )

  return (
    <Page
      title="Alerts"
      description="Webhook channels for Slack, Discord, or custom HTTP endpoints."
      actions={channels.data?.length ? newButton : undefined}
      className="max-w-4xl"
    >
      {channels.isPending ? (
        <Panel>
          <div className="space-y-2">
            {Array.from({ length: 2 }, (_, i) => (
              <Skeleton key={i} className="h-14" />
            ))}
          </div>
        </Panel>
      ) : channels.isError ? (
        <ErrorState error={channels.error} onRetry={() => void channels.refetch()} />
      ) : channels.data.length === 0 ? (
        <EmptyState icon={Bell} title="No alert channels" action={newButton}>
          Add a webhook to get notified on failed deploys, crashes, failed backups, or low disk space.
        </EmptyState>
      ) : (
        <Panel flush>
          <ul className="divide-y divide-border/80">
            {channels.data.map((c) => (
              <ChannelRow key={c.id} channel={c} onRemove={() => setRemoving(c)} />
            ))}
          </ul>
        </Panel>
      )}

      <CreateChannelDialog open={creating} onOpenChange={setCreating} onCreated={setCreated} />
      <SecretDialog created={created} onClose={() => setCreated(null)} />
      <RemoveDialog channel={removing} onClose={() => setRemoving(null)} />
    </Page>
  )
}

function useUpdateChannel(id: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (body: Schemas['UpdateAlertChannelRequest']) =>
      unwrap(api.PATCH('/api/v1/alerts/channels/{id}', { params: { path: { id } }, body })),
    onSuccess: (updated) => {
      queryClient.setQueryData(alertChannelsQuery.queryKey, (old) =>
        old?.map((c) => (c.id === updated.id ? updated : c)),
      )
    },
  })
}

function ChannelRow({ channel, onRemove }: { channel: Channel; onRemove: () => void }) {
  const queryClient = useQueryClient()
  const update = useUpdateChannel(channel.id)
  const test = useMutation({
    mutationFn: () =>
      unwrap(api.POST('/api/v1/alerts/channels/{id}/test', { params: { path: { id: channel.id } } })),
    onSuccess: async (result) => {
      if (result.ok) toast.success(`Test alert sent to ${channel.name}`)
      else toast.error(`${channel.name} did not accept the test alert`, { description: result.error ?? undefined })
      await queryClient.invalidateQueries({ queryKey: alertChannelsQuery.queryKey })
    },
  })
  const labels = EVENTS.filter((e) => channel.events.includes(e.value)).map((e) => e.label)

  return (
    <li className="flex flex-col gap-3 px-4 py-3.5 sm:flex-row sm:items-center">
      <div className="min-w-0 flex-1 space-y-1">
        <div className="flex flex-wrap items-center gap-2">
          <span className="truncate font-medium text-foreground">{channel.name}</span>
          <Badge variant="solid" color="gray" size="compact" className="font-mono text-muted-foreground">
            {channel.url_host}
          </Badge>
        </div>
        <p className="text-[12.5px] text-muted-foreground">
          {labels.length === EVENTS.length ? 'All events' : labels.join(', ')}
          {' · '}
          {channel.last_sent_at ? `last sent ${timeAgo(channel.last_sent_at)}` : 'never sent'}
        </p>
        {channel.last_error ? (
          <p className="text-[12.5px] break-words text-fail" role="status">
            Last delivery failed: {channel.last_error}
          </p>
        ) : null}
      </div>
      <div className="flex shrink-0 items-center gap-2">
        <Switch
          checked={channel.enabled}
          disabled={update.isPending}
          onToggle={() => update.mutate({ enabled: !channel.enabled })}
          aria-label={`Send alerts to ${channel.name}`}
        />
        <Button variant="outline" size="sm" disabled={test.isPending} onClick={() => test.mutate()}>
          {test.isPending ? <Spinner data-icon="inline-start" /> : <Send data-icon="inline-start" />}
          Test
        </Button>
        <Button variant="ghost" size="icon-sm" aria-label={`Remove ${channel.name}`} onClick={onRemove}>
          <Trash2 />
        </Button>
      </div>
    </li>
  )
}

function CreateChannelDialog({
  open,
  onOpenChange,
  onCreated,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  onCreated: (c: Schemas['CreatedAlertChannel']) => void
}) {
  const queryClient = useQueryClient()
  const [events, setEvents] = useState<AlertEvent[]>(ALL_EVENTS)

  const checkedIndices = useMemo(() => {
    const s = new Set<number>()
    EVENTS.forEach((ev, idx) => {
      if (events.includes(ev.value)) s.add(idx)
    })
    return s
  }, [events])

  const create = useMutation({
    mutationFn: (body: Schemas['CreateAlertChannelRequest']) =>
      unwrap(api.POST('/api/v1/alerts/channels', { body })),
    onSuccess: async (result) => {
      onOpenChange(false)
      setEvents(ALL_EVENTS)
      onCreated(result)
      await queryClient.invalidateQueries({ queryKey: alertChannelsQuery.queryKey })
    },
  })

  function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault()
    const form = new FormData(e.currentTarget)
    create.mutate({
      name: String(form.get('name') ?? ''),
      url: String(form.get('url') ?? '').trim(),
      events: EVENTS.map((x) => x.value).filter((v) => events.includes(v)),
    })
  }

  function toggle(value: AlertEvent) {
    setEvents((prev) => (prev.includes(value) ? prev.filter((v) => v !== value) : [...prev, value]))
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent size="lg">
        <form onSubmit={submit} className="contents">
          <DialogHeader>
            <DialogTitle>New alert channel</DialogTitle>
            <DialogDescription>Rig sends a signed JSON POST to this webhook URL.</DialogDescription>
          </DialogHeader>
          <FieldGroup>
            <Field>
              <FieldLabel htmlFor="alert-name">Name</FieldLabel>
              <Input id="alert-name" name="name" required maxLength={60} placeholder="Ops channel" autoFocus autoComplete="off" />
            </Field>
            <Field>
              <FieldLabel htmlFor="alert-url">Webhook URL</FieldLabel>
              <Input
                id="alert-url"
                name="url"
                type="url"
                required
                maxLength={2048}
                placeholder="https://hooks.slack.com/services/…"
                autoComplete="off"
                spellCheck={false}
              />
              <FieldDescription>Encrypted at rest; only the hostname is shown later.</FieldDescription>
            </Field>
            <fieldset className="grid gap-2">
              <legend className="mb-1 text-[13px] font-medium">Events</legend>
              <CheckboxGroup checkedIndices={checkedIndices} size="compact">
                {EVENTS.map((ev, index) => (
                  <CheckboxItem
                    key={ev.value}
                    index={index}
                    label={`${ev.label} — ${ev.hint}`}
                    checked={events.includes(ev.value)}
                    onToggle={() => toggle(ev.value)}
                  />
                ))}
              </CheckboxGroup>
            </fieldset>
          </FieldGroup>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={create.isPending || events.length === 0}>
              {create.isPending ? <Spinner data-icon="inline-start" /> : null}
              Add channel
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}

function SecretDialog({ created, onClose }: { created: Schemas['CreatedAlertChannel'] | null; onClose: () => void }) {
  return (
    <Dialog open={created !== null} onOpenChange={(open) => (open ? undefined : onClose())}>
      <DialogContent size="sm">
        <DialogHeader>
          <DialogTitle>Channel added</DialogTitle>
          <DialogDescription>
            Alerts to “{created?.channel.name}” include <code className="font-mono">x-rig-signature: sha256=…</code>.
            Copy this signing secret now; it is not shown again.
          </DialogDescription>
        </DialogHeader>
        <div className="flex items-center gap-1 rounded-lg border border-border/80 bg-surface/60 py-1 pr-1 pl-3">
          <code className="min-w-0 flex-1 overflow-x-auto py-1 text-[12.5px] whitespace-nowrap">
            {created?.signing_secret}
          </code>
          <CopyButton value={created?.signing_secret ?? ''} label="Copy signing secret" />
        </div>
        <DialogFooter>
          <Button onClick={onClose}>Done</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function RemoveDialog({ channel, onClose }: { channel: Channel | null; onClose: () => void }) {
  const queryClient = useQueryClient()
  const remove = useMutation({
    mutationFn: (id: string) => unwrap(api.DELETE('/api/v1/alerts/channels/{id}', { params: { path: { id } } })),
    meta: { success: 'Alert channel removed' },
    onSuccess: async () => {
      onClose()
      await queryClient.invalidateQueries({ queryKey: alertChannelsQuery.queryKey })
    },
  })

  return (
    <AlertDialog open={channel !== null} onOpenChange={(open) => (open ? undefined : onClose())}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Remove “{channel?.name}”?</AlertDialogTitle>
          <AlertDialogDescription>Rig will stop sending alerts to {channel?.url_host}.</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Keep channel</AlertDialogCancel>
          <AlertDialogAction
            variant="destructive"
            disabled={remove.isPending}
            onClick={() => channel && remove.mutate(channel.id)}
          >
            Remove channel
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
