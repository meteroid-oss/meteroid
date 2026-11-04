import { Badge, BadgeProps, Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@md/ui'

import { isUnreachable, nextProbeAt } from '@/features/settings/webhooks/endpointState'
import {
  WebhookDeliveryStatusEnum,
  WebhookEndpointDisabledReasonEnum,
  type WebhookEndpoint,
} from '@/rpc/api/webhooksout/v1/models_pb'
import { parseAndFormatDateTime } from '@/utils/date'

export const EndpointStatusBadge = ({ endpoint }: { endpoint: WebhookEndpoint }) => {
  if (endpoint.needsSetup) {
    return (
      <Badge
        variant="warning"
        size="sm"
        title="A secret header value is missing. Edit the endpoint to set it, then enable it."
      >
        Needs setup
      </Badge>
    )
  }
  if (isUnreachable(endpoint)) {
    const next = nextProbeAt(endpoint)
    return (
      <Badge
        variant="warning"
        size="sm"
        title={`The endpoint could not be reached several times in a row, so it is retried one event at a time. Next try: ${next ? parseAndFormatDateTime(next) : 'now'}. Events wait and are not lost.`}
      >
        Unreachable
      </Badge>
    )
  }
  if (!endpoint.disabled) {
    return (
      <Badge variant="success" size="sm">
        Active
      </Badge>
    )
  }
  const reason = endpoint.disabledReason
  const manual = reason === undefined || reason === WebhookEndpointDisabledReasonEnum.MANUAL
  return (
    <Badge variant={manual ? 'secondary' : 'destructive'} size="sm">
      Disabled
    </Badge>
  )
}

export const DELIVERY_STATUS_LABEL: Record<WebhookDeliveryStatusEnum, string> = {
  [WebhookDeliveryStatusEnum.PENDING]: 'Pending',
  [WebhookDeliveryStatusEnum.IN_FLIGHT]: 'In flight',
  [WebhookDeliveryStatusEnum.SUCCEEDED]: 'Succeeded',
  [WebhookDeliveryStatusEnum.FAILED]: 'Failed',
  [WebhookDeliveryStatusEnum.CANCELLED]: 'Cancelled',
}

const DELIVERY_STATUS_VARIANT: Record<WebhookDeliveryStatusEnum, BadgeProps['variant']> = {
  [WebhookDeliveryStatusEnum.PENDING]: 'secondary',
  [WebhookDeliveryStatusEnum.IN_FLIGHT]: 'outline',
  [WebhookDeliveryStatusEnum.SUCCEEDED]: 'success',
  [WebhookDeliveryStatusEnum.FAILED]: 'destructive',
  [WebhookDeliveryStatusEnum.CANCELLED]: 'ghost',
}

export const DeliveryStatusBadge = ({ status }: { status: WebhookDeliveryStatusEnum }) => (
  <Badge variant={DELIVERY_STATUS_VARIANT[status]} size="sm">
    {DELIVERY_STATUS_LABEL[status]}
  </Badge>
)

export const eventTypesSummary = (eventTypes: string[]) =>
  eventTypes.length === 0
    ? 'All events'
    : eventTypes.length === 1
      ? eventTypes[0]
      : `${eventTypes.length} event types`

/** The summary, listing every subscribed event type on hover once there are several. */
export const EventTypesSummary = ({ eventTypes }: { eventTypes: string[] }) => {
  const summary = eventTypesSummary(eventTypes)
  if (eventTypes.length < 2) return <span>{summary}</span>

  return (
    <TooltipProvider delayDuration={100}>
      <Tooltip>
        <TooltipTrigger asChild>
          <span className="cursor-default underline decoration-dotted underline-offset-4">
            {summary}
          </span>
        </TooltipTrigger>
        <TooltipContent className="max-w-[320px]">
          <ul className="space-y-0.5 font-mono text-xs">
            {[...eventTypes].sort().map(eventType => (
              <li key={eventType}>{eventType}</li>
            ))}
          </ul>
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  )
}
