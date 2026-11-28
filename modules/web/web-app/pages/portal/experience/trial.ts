import { billingPeriodNoun } from '@/features/checkout/utils/afterTrial'
import { TrialConfig_Status } from '@/rpc/api/subscriptions/v1/models_pb'

import { date, money } from './format'

import type { StatusBadge } from './format'
import type {
  PortalAfterTrial,
  PortalTrial,
  PortalTrialInvoice,
} from '@/rpc/portal/subscription/v1/subscription_pb'

/** Awaiting a decision, the trial's access goes on: the customer still sees a trial. */
export const trialBadge = (trial: PortalTrial): StatusBadge => {
  switch (trial.status) {
    case TrialConfig_Status.AWAITING_PAYMENT:
      return { label: 'Awaiting payment', tone: 'warn' }
    case TrialConfig_Status.AWAITING_DECISION:
      return { label: 'Trial', tone: 'warn' }
    default:
      return { label: `Trial · ends ${date(trial.endsOn)}`, tone: 'warn' }
  }
}

/** "€490.00/year + €40.00/month"; "From …" with usage on top; undefined when nothing recurs. */
export const afterTrialAmount = (after: PortalAfterTrial | undefined, usageBased: boolean) => {
  if (!after) return undefined
  if (after.recurring.length === 0) return usageBased ? 'Usage-based' : undefined
  const rate = after.recurring
    .map(entry => `${money(entry.amount, after.currency)}/${billingPeriodNoun(entry.interval)}`)
    .join(' + ')
  return usageBased ? `From ${rate}` : rate
}

/** The method on file as the customer knows it ("Visa •••• 4242"), else "On file". */
export const paymentMethodLabel = (trial: PortalTrial) => trial.paymentMethod || 'On file'

/** When it ended: as recorded, else its end. */
export const trialEndedOn = (trial: PortalTrial) => trial.endedOn || trial.endsOn

export const payInvoiceLabel = (invoice: PortalTrialInvoice) =>
  `Pay ${money(invoice.amountDue, invoice.currency)}`
