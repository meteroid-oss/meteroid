import { formatShortDay } from '@/features/subscriptions/utils/dates'
import { formatCurrency } from '@/lib/utils/numbers'
import { TrialAtEnd_Kind, TrialWithoutCard_Kind } from '@/rpc/api/plans/v1/models_pb'
import {
  SubscriptionFeeBillingPeriod,
  TrialConfig_Status,
} from '@/rpc/api/subscriptions/v1/models_pb'

import type { PlanNamer } from '@/features/plans/trial/trialTerms'
import type { FirstChargeEstimate, TrialConfig } from '@/rpc/api/subscriptions/v1/models_pb'

export const TRIAL_STATUS: Record<
  TrialConfig_Status,
  { label: string; variant: 'info' | 'success' | 'secondary' | 'warning' }
> = {
  [TrialConfig_Status.ACTIVE]: { label: 'Trial', variant: 'info' },
  [TrialConfig_Status.CONVERTED]: { label: 'Converted', variant: 'success' },
  [TrialConfig_Status.CANCELLED]: { label: 'Cancelled', variant: 'secondary' },
  [TrialConfig_Status.SWITCHED]: { label: 'Switched', variant: 'secondary' },
  [TrialConfig_Status.AWAITING_PAYMENT]: { label: 'Awaiting payment', variant: 'warning' },
  [TrialConfig_Status.AWAITING_DECISION]: { label: 'Awaiting decision', variant: 'warning' },
}

const PERIOD_SUFFIX: Partial<Record<SubscriptionFeeBillingPeriod, string>> = {
  [SubscriptionFeeBillingPeriod.MONTHLY]: '/month',
  [SubscriptionFeeBillingPeriod.QUARTERLY]: '/quarter',
  [SubscriptionFeeBillingPeriod.SEMIANNUAL]: '/half-year',
  [SubscriptionFeeBillingPeriod.YEARLY]: '/year',
}

/** "/month" when every recurring fee bills on one period, else nothing. */
export const periodSuffix = (periods: SubscriptionFeeBillingPeriod[]) => {
  const recurring = new Set(periods.filter(p => p !== SubscriptionFeeBillingPeriod.ONE_TIME))
  return recurring.size === 1 ? (PERIOD_SUFFIX[[...recurring][0]] ?? '') : ''
}

/** "€49.00/month from Dec 15". */
export const firstChargeText = (
  estimate: FirstChargeEstimate,
  periods: SubscriptionFeeBillingPeriod[]
) => {
  const amount = formatCurrency(estimate.amount, estimate.currency)
  return `${amount}${periodSuffix(periods)} from ${formatShortDay(estimate.on)}`
}

interface Naming {
  isFreePlan: boolean
  planName: PlanNamer
  ownName: string
}

const switchedTo = (planVersionId: string | undefined, planName: PlanNamer) =>
  `Switches to ${planName(planVersionId) ?? 'another plan'}`

export const atEndText = (trial: TrialConfig, { isFreePlan, planName, ownName }: Naming) => {
  if (isFreePlan) return `Stays on ${ownName}`
  switch (trial.atEnd?.kind ?? TrialAtEnd_Kind.CONVERT) {
    case TrialAtEnd_Kind.MANUAL:
      return 'Account owner decides'
    case TrialAtEnd_Kind.CANCEL:
      return 'Cancels'
    case TrialAtEnd_Kind.SWITCH:
      return switchedTo(trial.atEnd?.planVersionId, planName)
    default:
      return 'Converts'
  }
}

/** What a converting trial charged automatically does without a payment method; else none. */
export const withoutCardText = (
  trial: TrialConfig,
  { chargedAutomatically, isFreePlan, planName }: Naming & { chargedAutomatically: boolean }
) => {
  if (isFreePlan || !chargedAutomatically) return undefined
  if ((trial.atEnd?.kind ?? TrialAtEnd_Kind.CONVERT) !== TrialAtEnd_Kind.CONVERT) return undefined
  switch (trial.withoutCard?.kind ?? TrialWithoutCard_Kind.AWAIT_PAYMENT) {
    case TrialWithoutCard_Kind.CANCEL:
      return 'Cancels'
    case TrialWithoutCard_Kind.SWITCH:
      return switchedTo(trial.withoutCard?.planVersionId, planName)
    default:
      return 'Awaits payment'
  }
}

/** What ending a trial now does, as its preview tells it. */
export const END_OUTCOME: Partial<Record<TrialConfig_Status, string>> = {
  [TrialConfig_Status.CONVERTED]: 'Billing starts',
  [TrialConfig_Status.CANCELLED]: 'Subscription ends',
  [TrialConfig_Status.SWITCHED]: 'Switches plan',
}
