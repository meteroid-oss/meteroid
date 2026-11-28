import { formatDateOnly } from '@/lib/utils/date'
import { formatCurrency } from '@/lib/utils/numbers'
import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'

import type { SubscriptionComponent } from '@/rpc/api/subscriptions/v1/models_pb'
import type { AfterTrial } from '@/rpc/portal/checkout/v1/models_pb'

// The unit of time a recurring amount is owed over, e.g. "€50 / quarter".
export const billingPeriodNoun = (period: BillingPeriod | undefined): string => {
  switch (period) {
    case BillingPeriod.QUARTERLY:
      return 'quarter'
    case BillingPeriod.SEMIANNUAL:
      return 'half-year'
    case BillingPeriod.ANNUAL:
      return 'year'
    default:
      return 'month'
  }
}

export const billsUsage = (components: SubscriptionComponent[]) =>
  components.some(c => c.fee?.fee.case === 'usage' || c.fee?.fee.case === 'capacity')

/** "then €490.00/year + €40.00/month from Dec 15, 2026": recurring fees, "from" a usage plan. */
export const afterTrialText = (after: AfterTrial | undefined, usage: boolean) => {
  if (!after || (after.recurring.length === 0 && !usage)) return undefined
  const on = formatDateOnly(after.startsOn)
  if (after.recurring.length === 0) return `then usage-based from ${on}`
  const rate = after.recurring
    .map(({ amount, interval }) => {
      return `${formatCurrency(amount, after.currency)}/${billingPeriodNoun(interval)}`
    })
    .join(' + ')
  return usage ? `then from ${rate}, starting ${on}` : `then ${rate} from ${on}`
}
