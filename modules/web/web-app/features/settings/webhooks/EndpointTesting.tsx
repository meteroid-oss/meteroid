import { json } from '@codemirror/lang-json'
import {
  Button,
  Label,
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from '@md/ui'
import { githubDark, githubLight } from '@uiw/codemirror-theme-github'
import CodeMirror from '@uiw/react-codemirror'
import { useMemo, useState } from 'react'
import { toast } from 'sonner'

import { AttemptOutput } from '@/features/settings/webhooks/AttemptOutput'
import { DeliveryStatusBadge } from '@/features/settings/webhooks/badges'
import { useWebhooks } from '@/features/settings/webhooks/useWebhooks'
import { useQuery } from '@/lib/connectrpc'
import {
  WebhookDeliveryStatusEnum,
  type WebhookEndpoint,
  type WebhookEventType,
} from '@/rpc/api/webhooksout/v1/models_pb'
import {
  getWebhookDelivery,
  listWebhookEventTypes,
} from '@/rpc/api/webhooksout/v1/webhooksout-WebhooksService_connectquery'
import { parseAndFormatDateTime } from '@/utils/date'
import { useTheme } from 'providers/ThemeProvider'

const GENERIC = 'webhook.test'

const GENERIC_EXAMPLE = JSON.stringify(
  {
    id: 'evt_…',
    type: GENERIC,
    data: { endpoint_id: '…' },
    timestamp: '…',
  },
  null,
  2
)

const DONE = new Set([
  WebhookDeliveryStatusEnum.SUCCEEDED,
  WebhookDeliveryStatusEnum.FAILED,
  WebhookDeliveryStatusEnum.CANCELLED,
])

interface Props {
  endpoint: WebhookEndpoint
  onOpenDelivery: (deliveryId: string) => void
}

/** Sends an example event to the endpoint and shows how it answered. */
export const EndpointTesting = ({ endpoint, onOpenDelivery }: Props) => {
  const { isDarkMode } = useTheme()
  const { sendTest } = useWebhooks()
  const eventTypesQuery = useQuery(listWebhookEventTypes)
  const [eventType, setEventType] = useState(GENERIC)
  const [deliveryId, setDeliveryId] = useState<string | null>(null)

  const groups = useMemo(() => {
    const byGroup = new Map<string, WebhookEventType[]>()
    for (const type of eventTypesQuery.data?.eventTypes ?? []) {
      const existing = byGroup.get(type.group)
      if (existing) existing.push(type)
      else byGroup.set(type.group, [type])
    }
    return [...byGroup.entries()].sort(([a], [b]) => a.localeCompare(b))
  }, [eventTypesQuery.data?.eventTypes])

  const example =
    eventType === GENERIC
      ? GENERIC_EXAMPLE
      : (eventTypesQuery.data?.eventTypes.find(t => t.name === eventType)?.exampleJson ?? '')

  const resultQuery = useQuery(
    getWebhookDelivery,
    { id: deliveryId ?? '' },
    {
      enabled: deliveryId != null,
      refetchInterval: query =>
        DONE.has(query.state.data?.delivery?.status ?? WebhookDeliveryStatusEnum.PENDING)
          ? false
          : 1000,
    }
  )
  const result = resultQuery.data?.delivery
  const lastAttempt = [...(resultQuery.data?.attempts ?? [])].sort(
    (a, b) => b.attemptNumber - a.attemptNumber
  )[0]

  const send = () =>
    sendTest.mutate(
      { endpointId: endpoint.id, eventType: eventType === GENERIC ? undefined : eventType },
      {
        onSuccess: res => setDeliveryId(res.delivery?.id ?? null),
        onError: e => toast.error(e.message),
      }
    )

  return (
    <div className="space-y-5">
      <div className="space-y-1">
        <h3 className="text-sm font-medium">Send an example event</h3>
        <p className="text-sm text-muted-foreground">
          We send an example payload to this endpoint, signed like a real event. Nothing changes in
          your billing data. Example events carry made-up ids, so make sure your receiver does not
          act on them.
        </p>
      </div>

      <div className="space-y-1.5">
        <Label>Event type</Label>
        <Select value={eventType} onValueChange={setEventType}>
          <SelectTrigger className="w-[320px]">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={GENERIC}>webhook.test (generic test event)</SelectItem>
            {groups.map(([group, types]) => (
              <SelectGroup key={group}>
                <SelectLabel className="capitalize">{group}</SelectLabel>
                {types.map(type => (
                  <SelectItem key={type.name} value={type.name}>
                    {type.name}
                  </SelectItem>
                ))}
              </SelectGroup>
            ))}
          </SelectContent>
        </Select>
      </div>

      <div className="space-y-1.5">
        <Label>Payload</Label>
        <div className="overflow-hidden rounded-lg border border-border">
          <CodeMirror
            value={example}
            extensions={[json()]}
            theme={isDarkMode ? githubDark : githubLight}
            editable={false}
            readOnly
            maxHeight="280px"
            basicSetup={{ foldGutter: false, highlightActiveLine: false }}
          />
        </div>
      </div>

      <Button size="sm" disabled={endpoint.disabled || sendTest.isPending} onClick={send}>
        Send example
      </Button>
      {endpoint.disabled && (
        <p className="text-xs text-muted-foreground">Enable the endpoint to send a test.</p>
      )}

      {result && (
        <section className="space-y-2 rounded-lg border border-border p-4">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2 text-sm font-medium">
              Result <DeliveryStatusBadge status={result.status} />
            </div>
            <Button variant="ghost" size="sm" onClick={() => onOpenDelivery(result.id)}>
              Open delivery →
            </Button>
          </div>
          {lastAttempt ? (
            <>
              <dl className="grid grid-cols-3 gap-4 text-[13px]">
                <div>
                  <dt className="text-xs text-muted-foreground">Response</dt>
                  <dd>{lastAttempt.responseStatus ?? 'No response'}</dd>
                </div>
                <div>
                  <dt className="text-xs text-muted-foreground">Duration</dt>
                  <dd>{lastAttempt.durationMs.toLocaleString()} ms</dd>
                </div>
                <div>
                  <dt className="text-xs text-muted-foreground">Sent</dt>
                  <dd>{parseAndFormatDateTime(lastAttempt.attemptedAt, 'sec')}</dd>
                </div>
              </dl>
              <AttemptOutput attempt={lastAttempt} />
            </>
          ) : (
            <p className="text-sm text-muted-foreground">Sending…</p>
          )}
        </section>
      )}

      <p className="text-xs text-muted-foreground">
        Tests also appear in the Overview delivery log, tagged “test”.
      </p>
    </div>
  )
}
