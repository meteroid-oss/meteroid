import Decimal from 'decimal.js'

import { formatPrice } from '@/features/plans/pricecomponents/utils'
import { CreditPackExpiry_Unit } from '@/rpc/api/prices/v1/models_pb'
import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'

import type { CreditPackExpiry, CreditPricing } from '@/rpc/api/prices/v1/models_pb'

export const decimalOf = (amount: string | undefined) => new Decimal(amount || 0)

export const cadenceAdverb = (cadence: BillingPeriod): string => {
  switch (cadence) {
    case BillingPeriod.MONTHLY:
      return 'monthly'
    case BillingPeriod.QUARTERLY:
      return 'quarterly'
    case BillingPeriod.SEMIANNUAL:
      return 'semiannually'
    case BillingPeriod.ANNUAL:
      return 'annually'
  }
}

export const cadencePeriod = (cadence: BillingPeriod): string => {
  switch (cadence) {
    case BillingPeriod.MONTHLY:
      return 'month'
    case BillingPeriod.QUARTERLY:
      return 'quarter'
    case BillingPeriod.SEMIANNUAL:
      return '6 months'
    case BillingPeriod.ANNUAL:
      return 'year'
  }
}

/** "month", or "month or year" when a component is offered on several terms. */
export const cadencesPeriod = (cadences: BillingPeriod[]) =>
  [...new Set(cadences)].map(cadencePeriod).join(' or ')

/** A pack the plan includes at no charge: the engine grants it as promotional credit. */
export const isIncludedPack = (pricing: Pick<CreditPricing, 'unitPrice'>) =>
  decimalOf(pricing.unitPrice).isZero()

/** "monthly", or "monthly or annually" when a component is offered on several terms. */
export const cadencesAdverb = (cadences: BillingPeriod[]) =>
  [...new Set(cadences)].map(cadenceAdverb).join(' or ')

export const formatCreditQuantity = (amount: Decimal, precision = 2) =>
  new Intl.NumberFormat(undefined, { maximumFractionDigits: precision }).format(amount.toNumber())

export type CreditNames = { name: string; pluralName: string }

/** "500 fast requests", in the unit's own name; "credits" while the unit is unknown. */
export const creditsLabel = (amount: Decimal, precision?: number, names?: CreditNames) =>
  `${formatCreditQuantity(amount, precision)} ${
    amount.eq(1) ? (names?.name ?? 'credit') : (names?.pluralName ?? 'credits')
  }`

/** What an amount is expressed in: money, or a credit unit worth `rate` of `currency` per credit. */
export type Denomination =
  | { kind: 'money'; currency: string }
  | {
      kind: 'credit'
      currency: string
      /** What one credit is worth in `currency`; none without a money value. */
      rate?: string
      precision?: number
      names?: CreditNames
    }

export const formatAmount = (denomination: Denomination, amount: string) =>
  denomination.kind === 'money'
    ? formatPrice(denomination.currency)(amount)
    : creditsLabel(decimalOf(amount), denomination.precision, denomination.names)

export const expirySpan = (expiry: CreditPackExpiry) => {
  const unit = expiry.unit === CreditPackExpiry_Unit.DAY ? 'day' : 'month'
  return `${expiry.value} ${unit}${expiry.value === 1 ? '' : 's'}`
}

export const hasDuration = (expiry: CreditPackExpiry | undefined): expiry is CreditPackExpiry =>
  expiry?.unit === CreditPackExpiry_Unit.DAY || expiry?.unit === CreditPackExpiry_Unit.MONTH

export const humanizeKey = (key: string) => {
  const words = key.replace(/[_-]+/g, ' ').trim()
  return words.charAt(0).toUpperCase() + words.slice(1)
}
