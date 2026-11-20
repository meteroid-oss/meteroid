import { formatDistanceToNow } from 'date-fns'
import Decimal from 'decimal.js'

import { currencyPrecision } from '@/lib/data/currency-precision'
import { parseUtc } from '@/lib/utils/date'
import { formatCurrency } from '@/lib/utils/numbers'
import { QuoteTotals } from '@/rpc/api/quotes/v1/models_pb'
import { SubscriptionFeeBillingPeriod } from '@/rpc/api/subscriptions/v1/models_pb'

/** A decimal string in major units, as the server's totals carry them. */
export const formatMajorAmount = (amount: string, currency: string) => {
  const minor = new Decimal(amount || '0').mul(new Decimal(10).pow(currencyPrecision(currency)))
  return formatCurrency(minor.toNumber(), currency, { trimWholeAmounts: true })
}

const periodSuffix: Record<SubscriptionFeeBillingPeriod, string> = {
  [SubscriptionFeeBillingPeriod.ONE_TIME]: 'once',
  [SubscriptionFeeBillingPeriod.MONTHLY]: '/ month',
  [SubscriptionFeeBillingPeriod.QUARTERLY]: '/ quarter',
  [SubscriptionFeeBillingPeriod.SEMIANNUAL]: '/ 6 months',
  [SubscriptionFeeBillingPeriod.YEARLY]: '/ year',
}

/** "€2,418.75 / month + €300 / year", with "+ usage" when usage lines are billed on top. */
export const recurringTotalLabel = (totals: QuoteTotals): string | undefined => {
  const parts = totals.recurring.map(total =>
    `${formatMajorAmount(total.amount, totals.currency)} ${periodSuffix[total.period] ?? ''}`.trim()
  )
  if (parts.length === 0) return totals.hasUsage ? 'Usage-based' : undefined
  return totals.hasUsage ? `${parts.join(' + ')} + usage` : parts.join(' + ')
}

/** A naive UTC timestamp of the API as "Nov 12". */
export const shortDate = (value: string) =>
  parseUtc(value).toLocaleDateString('en-US', { month: 'short', day: 'numeric' })

export const fromNow = (value: string) => formatDistanceToNow(parseUtc(value), { addSuffix: true })

export const initials = (name: string, email: string) => {
  const words = (name.trim() || email.split('@')[0]).split(/[\s._-]+/).filter(Boolean)
  return words
    .slice(0, 2)
    .map(word => word.charAt(0).toUpperCase())
    .join('')
}

export const shortHash = (hash?: string) => (hash ? hash.slice(0, 12) : '—')
