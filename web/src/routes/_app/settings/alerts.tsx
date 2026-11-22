import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { Bell, Plus, Send, Trash2 } from 'lucide-react'
import { useState, type FormEvent } from 'react'
import { toast } from 'sonner'
import { api, unwrap, type Schemas } from '@/api/client'
import { alertChannelsQuery } from '@/api/queries'
import { CopyButton } from '@/components/app/copy-button'
import { Page, Panel } from '@/components/app/page'
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
import { Checkbox } from '@/components/ui/checkbox'
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
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { Switch } from '@/components/ui/switch'
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
      description="Rig posts a JSON message to each channel when something needs attention. Slack and Discord webhook URLs work as they are."
      actions={channels.data?.length ? newButton : undefined}
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
          Add a webhook to hear about failed deploys, crashes, failed backups and low disk space.
        </EmptyState>
      ) : (
        <Panel flush>
          <ul className="divide-y">
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
    <li className="flex flex-col gap-3 px-4 py-3 sm:flex-row sm:items-center">
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="truncate font-medium">{channel.name}</span>
          <code className="truncate text-[0.8125rem] text-muted-foreground">{channel.url_host}</code>
        </div>
        <p className="mt-0.5 text-[0.8125rem] text-muted-foreground">
          {labels.length === EVENTS.length ? 'Every event' : labels.join(', ')}
          {' · '}
          {channel.last_sent_at ? `last sent ${timeAgo(channel.last_sent_at)}` : 'nothing sent yet'}
        </p>
        {channel.last_error ? (
          <p className="mt-1 text-[0.8125rem] break-words text-fail" role="status">
            Last delivery failed: {channel.last_error}
          </p>
        ) : null}
      </div>
      <div className="flex shrink-0 items-center gap-1">
        <Switch
          checked={channel.enabled}
          disabled={update.isPending}
          onCheckedChange={(enabled) => update.mutate({ enabled })}
          aria-label={`Send alerts to ${channel.name}`}
          className="mr-2"
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

  function toggle(value: AlertEvent, on: boolean) {
    setEvents((prev) => (on ? [...prev, value] : prev.filter((v) => v !== value)))
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <form onSubmit={submit} className="contents">
          <DialogHeader>
            <DialogTitle>New alert channel</DialogTitle>
            <DialogDescription>Rig sends a signed POST to this URL for the events you pick.</DialogDescription>
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
              <FieldDescription>Stored encrypted. Only its host is shown afterwards.</FieldDescription>
            </Field>
            <fieldset className="space-y-2">
              <legend className="mb-2 text-sm font-medium">Events</legend>
              {EVENTS.map((ev) => (
                <label key={ev.value} className="flex cursor-pointer items-start gap-2.5">
                  <Checkbox
                    className="mt-0.5"
                    checked={events.includes(ev.value)}
                    onCheckedChange={(on) => toggle(ev.value, on)}
                  />
                  <span className="text-sm leading-tight">
                    {ev.label}
                    <span className="block text-[0.8125rem] text-muted-foreground">{ev.hint}</span>
                  </span>
                </label>
              ))}
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
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Channel added</DialogTitle>
          <DialogDescription>
            Every alert to “{created?.channel.name}” carries an <code>x-rig-signature: sha256=…</code> header, an
            HMAC of the body with this secret. Copy it now if the receiver checks signatures; it is not shown again.
          </DialogDescription>
        </DialogHeader>
        <div className="flex items-center gap-1 rounded-md border bg-muted/50 py-1 pr-1 pl-3">
          <code className="min-w-0 flex-1 overflow-x-auto py-1 text-[0.8125rem] whitespace-nowrap">
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
          <AlertDialogDescription>Rig stops sending alerts to {channel?.url_host}.</AlertDialogDescription>
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
