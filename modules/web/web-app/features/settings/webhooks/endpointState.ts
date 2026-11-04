import {
  WebhookEndpointDisabledReasonEnum,
  type WebhookEndpoint,
} from '@/rpc/api/webhooksout/v1/models_pb'
import { parseAndFormatDateTime } from '@/utils/date'

/**
 * Unreachable several times in a row, so only probed now and then. `pausedUntil` stays set
 * once the pause is over, until the endpoint answers: the state holds between two probes.
 */
export const isUnreachable = (endpoint: WebhookEndpoint) =>
  !endpoint.disabled && endpoint.pausedUntil != null

/** When the next probe goes out, or undefined when it is due now. */
export const nextProbeAt = (endpoint: WebhookEndpoint): string | undefined =>
  endpoint.pausedUntil != null && new Date(endpoint.pausedUntil) > new Date()
    ? endpoint.pausedUntil
    : undefined

/** Rows that need the customer's attention sort first in the list. */
export const needsAttention = (endpoint: WebhookEndpoint) =>
  endpoint.needsSetup ||
  isUnreachable(endpoint) ||
  (endpoint.disabled &&
    endpoint.disabledReason !== undefined &&
    endpoint.disabledReason !== WebhookEndpointDisabledReasonEnum.MANUAL)

const DISABLED_REASON_LINE: Record<WebhookEndpointDisabledReasonEnum, string> = {
  [WebhookEndpointDisabledReasonEnum.MANUAL]: 'Turned off',
  [WebhookEndpointDisabledReasonEnum.AUTO_FAILURES]: 'Disabled after repeated failures',
  [WebhookEndpointDisabledReasonEnum.GONE]: 'Returned 410 Gone',
}

/** The short reason shown under the status badge, if the state has one. */
export const statusReason = (endpoint: WebhookEndpoint): string | undefined => {
  if (endpoint.needsSetup) return undefined
  if (isUnreachable(endpoint)) {
    return endpoint.failingSince
      ? `Unreachable since ${parseAndFormatDateTime(endpoint.failingSince)}`
      : 'Unreachable'
  }
  if (endpoint.disabled && endpoint.disabledReason !== undefined) {
    return DISABLED_REASON_LINE[endpoint.disabledReason]
  }
  return undefined
}

/** The most recent delivery attempt that finished, success or failure. */
export const lastDeliveryAt = (endpoint: WebhookEndpoint): string | undefined => {
  const times = [endpoint.lastSuccessAt, endpoint.lastFailureAt].filter(
    (t): t is string => t != null
  )
  return times.sort().at(-1)
}

export const timeAgo = (iso: string): string => {
  const seconds = Math.max(0, Math.round((Date.now() - new Date(iso).getTime()) / 1000))
  if (seconds < 60) return 'just now'
  const minutes = Math.round(seconds / 60)
  if (minutes < 60) return `${minutes} min ago`
  const hours = Math.round(minutes / 60)
  if (hours < 24) return `${hours} h ago`
  return parseAndFormatDateTime(iso)
}

/** Deliveries by outcome, as the stats endpoints count them. */
export interface DeliveryOutcomes {
  succeeded: number
  failed: number
  retrying: number
  cancelled: number
}

export const outcomesOf = (stats: {
  succeeded: bigint
  failed: bigint
  retrying: bigint
  cancelled: bigint
}): DeliveryOutcomes => ({
  succeeded: Number(stats.succeeded),
  failed: Number(stats.failed),
  retrying: Number(stats.retrying),
  cancelled: Number(stats.cancelled),
})

export const outcomesTotal = (o: DeliveryOutcomes) =>
  o.succeeded + o.failed + o.retrying + o.cancelled

/** Share of deliveries that went through. One still being retried has not, yet. */
export const successRate = (outcomes: DeliveryOutcomes): string | undefined => {
  const total = outcomesTotal(outcomes)
  if (total === 0) return undefined
  const rate = (outcomes.succeeded / total) * 100
  return `${rate >= 99.95 ? '100' : rate.toFixed(1)}%`
}
