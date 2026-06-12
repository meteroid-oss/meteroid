import { ActivityEntry, ActorType } from '@/rpc/api/activity/v1/activity_pb'

export type { ActivityEntry, ActorType }

// Activity types with a custom renderer; unknown types fall back to a generic row.
export const KNOWN_ACTIVITY_TYPES = [
  'customer.created',
  'customer.updated',
  'customer.archived',
  'customer.unarchived',
  'invoice.created',
  'invoice.finalized',
  'invoice.paid',
  'invoice.voided',
  'invoice.consolidated',
  'product.created',
  'product.updated',
  'product.archived',
  'plan.created',
  'plan.published',
  'plan.archived',
  'plan.draft_discarded',
  'add_on.created',
  'add_on.updated',
  'add_on.archived',
  'coupon.created',
  'coupon.updated',
  'coupon.archived',
  'billable_metric.created',
  'billable_metric.updated',
  'billable_metric.archived',
  'subscription.created',
  'credit_note.created',
  'credit_note.finalized',
  'credit_note.voided',
  'quote.created',
  'quote.updated',
  'quote.accepted',
  'quote.converted',
  'quote.published',
  'quote.declined',
  'quote.viewed',
  'quote.signature_added',
  'quote.cancelled',
  'quote.sent',
  'customer.logged_in',
  'subscription.paused',
  'subscription.cancellation_scheduled',
  'subscription.cancelled',
  'subscription.cancellation_undone',
  'subscription.plan_change_scheduled',
  'subscription.plan_change_cancelled',
  'subscription.plan_changed',
  'subscription.amendment_scheduled',
  'subscription.amendment_cancelled',
  'subscription.amended',
  'api_token.created',
  'api_token.revoked',
  'connector.connected',
  'connector.disconnected',
  'entity.email_sent',
  'entity.field_changed',
  'entity.updated',
] as const

export type KnownActivityType = (typeof KNOWN_ACTIVITY_TYPES)[number]

export function parseMetadata(entry: ActivityEntry): Record<string, unknown> {
  if (!entry.metadataJson) return {}
  try {
    return JSON.parse(entry.metadataJson) as Record<string, unknown>
  } catch {
    return {}
  }
}
