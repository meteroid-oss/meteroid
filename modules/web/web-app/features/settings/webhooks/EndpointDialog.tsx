import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Input,
  Label,
} from '@md/ui'
import { useEffect, useState } from 'react'
import { toast } from 'sonner'

import { EventTypeSelect } from '@/features/settings/webhooks/EventTypeSelect'
import {
  type HeaderDraft,
  HeadersEditor,
  draftsFrom,
  headersError,
  toHeaderInputs,
} from '@/features/settings/webhooks/HeadersEditor'
import { useWebhooks } from '@/features/settings/webhooks/useWebhooks'
import { useTenant } from '@/hooks/useTenant'
import { TenantEnvironmentEnum } from '@/rpc/api/tenants/v1/models_pb'
import { type WebhookEndpoint } from '@/rpc/api/webhooksout/v1/models_pb'

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
  editing?: WebhookEndpoint
  onCreated: (endpointId: string, secret: string) => void
}

const isValidUrl = (raw: string) => {
  try {
    const url = new URL(raw)
    return url.protocol === 'https:' || url.protocol === 'http:'
  } catch {
    return false
  }
}

export const EndpointDialog = ({ open, onOpenChange, editing, onCreated }: Props) => {
  const { create, update } = useWebhooks()
  const { tenant } = useTenant()
  // Production endpoints get real data: no public test inbox suggested there.
  const suggestTestInbox = tenant != null && tenant.environment !== TenantEnvironmentEnum.PRODUCTION
  const isEdit = editing != null

  const [url, setUrl] = useState('')
  const [description, setDescription] = useState('')
  const [allEvents, setAllEvents] = useState(true)
  const [eventTypes, setEventTypes] = useState<string[]>([])
  const [headers, setHeaders] = useState<HeaderDraft[]>([])

  useEffect(() => {
    if (!open) return
    setUrl(editing?.url ?? '')
    setDescription(editing?.description ?? '')
    setAllEvents(editing == null || editing.eventTypes.length === 0)
    setEventTypes(editing?.eventTypes ?? [])
    setHeaders(draftsFrom(editing?.headers ?? []))
  }, [open, editing])

  const trimmedUrl = url.trim()
  const urlTouched = trimmedUrl.length > 0
  const headerProblem = headersError(headers)
  const canSubmit =
    isValidUrl(trimmedUrl) &&
    (allEvents || eventTypes.length > 0) &&
    headerProblem === null &&
    !create.isPending &&
    !update.isPending

  const submit = () => {
    if (!canSubmit) return
    const selected = allEvents ? [] : eventTypes
    const desc = description.trim() === '' ? undefined : description.trim()

    if (isEdit && editing) {
      update.mutate(
        {
          id: editing.id,
          url: trimmedUrl,
          description: desc,
          eventTypes: { eventTypes: selected },
          headers: { headers: toHeaderInputs(headers) },
          // Setting up a migrated endpoint means it should start receiving events.
          disabled: editing.needsSetup ? false : undefined,
        },
        {
          onSuccess: () => {
            toast.success(editing.needsSetup ? 'Endpoint enabled' : 'Endpoint updated')
            onOpenChange(false)
          },
          onError: e => toast.error(e.message),
        }
      )
      return
    }

    create.mutate(
      {
        url: trimmedUrl,
        description: desc,
        eventTypes: selected,
        headers: toHeaderInputs(headers),
      },
      {
        onSuccess: res => {
          onOpenChange(false)
          if (res.endpoint) onCreated(res.endpoint.id, res.secret)
        },
        onError: e => toast.error(e.message),
      }
    )
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[540px]">
        <DialogHeader>
          <DialogTitle>{isEdit ? 'Edit endpoint' : 'Add webhook endpoint'}</DialogTitle>
          <DialogDescription>
            We POST a signed JSON event to this URL for every event you choose.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div className="space-y-1.5">
            <Label htmlFor="webhook-url">Endpoint URL</Label>
            <Input
              id="webhook-url"
              value={url}
              onChange={e => setUrl(e.target.value)}
              placeholder="https://example.com/hooks/meteroid"
            />
            {urlTouched && !isValidUrl(trimmedUrl) && (
              <p className="text-xs text-destructive">Enter a valid http(s) URL.</p>
            )}
            {trimmedUrl.toLowerCase().startsWith('http://') && (
              <p className="text-xs text-muted-foreground">
                Use an https:// URL: events carry billing and customer data. Plain http is only
                accepted on instances set up for local development. To receive events on your own
                machine, expose it through a tunnel (ngrok, Cloudflare Tunnel…) and use its https
                URL.
              </p>
            )}
            {suggestTestInbox && !isEdit && !urlTouched && (
              <p className="text-xs text-muted-foreground">
                No server yet? Create a temporary inbox on{' '}
                <a
                  href="https://webhook.site"
                  target="_blank"
                  rel="noreferrer"
                  className="underline underline-offset-2 hover:text-foreground"
                >
                  webhook.site
                </a>{' '}
                and paste its URL here. Anyone with that link can read what we send, so use it with
                test data only.
              </p>
            )}
          </div>

          <div className="space-y-1.5">
            <Label htmlFor="webhook-description">Description (optional)</Label>
            <Input
              id="webhook-description"
              value={description}
              onChange={e => setDescription(e.target.value)}
              placeholder="Billing events to our ERP"
            />
          </div>

          <div className="space-y-1.5">
            <Label>Event types</Label>
            <EventTypeSelect
              allEvents={allEvents}
              onAllEventsChange={setAllEvents}
              value={eventTypes}
              onChange={setEventTypes}
            />
          </div>

          <div className="space-y-1.5">
            <Label>Custom headers (optional)</Label>
            <p className="text-xs text-muted-foreground">
              Sent with every delivery, e.g. an API key your receiver checks.
            </p>
            <HeadersEditor value={headers} onChange={setHeaders} />
            {headers.length > 0 && headerProblem && (
              <p className="text-xs text-destructive">{headerProblem}</p>
            )}
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" type="button" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button type="button" disabled={!canSubmit} onClick={submit}>
            {isEdit ? 'Save changes' : 'Add endpoint'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
