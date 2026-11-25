import { SubscriptionFeeBillingPeriod } from '@/rpc/api/subscriptions/v1/models_pb'

/** A level of a usage-levels charge: `included` units for `rate` a period. */
export interface LevelLike {
  included: bigint
  rate: string
}

/** How a level's size reads: the metric's unit, or the currency when it counts money. */
export interface LevelUnit {
  /** The metric counts money in the subscription's currency ("€250K"). */
  money?: boolean
  plural?: string | null
}

const PER: Partial<Record<SubscriptionFeeBillingPeriod, string>> = {
  [SubscriptionFeeBillingPeriod.MONTHLY]: 'month',
  [SubscriptionFeeBillingPeriod.QUARTERLY]: 'quarter',
  [SubscriptionFeeBillingPeriod.SEMIANNUAL]: '6 months',
  [SubscriptionFeeBillingPeriod.YEARLY]: 'year',
}

const compactCount = new Intl.NumberFormat('en-US', {
  notation: 'compact',
  maximumFractionDigits: 1,
})

const amount = (value: string, currency: string, compact = false) => {
  const n = Number(value)
  try {
    return n.toLocaleString('en-US', {
      style: 'currency',
      currency,
      currencyDisplay: 'narrowSymbol',
      ...(compact
        ? { notation: 'compact', minimumFractionDigits: 0, maximumFractionDigits: 1 }
        : { minimumFractionDigits: Number.isInteger(n) ? 0 : 2, maximumFractionDigits: 2 }),
    })
  } catch {
    return `${compact ? compactCount.format(n) : value} ${currency}`
  }
}

/** "250K", "€250K" for money, "250K requests" with a unit. */
export const levelSize = (included: bigint, currency: string, unit?: LevelUnit): string => {
  if (unit?.money) return amount(included.toString(), currency, true)
  const count = compactCount.format(Number(included))
  return unit?.plural ? `${count} ${unit.plural}` : count
}

/** "Up to €250K — €899 / month". */
export const levelWords = (
  level: LevelLike,
  currency: string,
  period: SubscriptionFeeBillingPeriod,
  unit?: LevelUnit
): string => {
  const per = PER[period]
  return `Up to ${levelSize(level.included, currency, unit)} — ${amount(level.rate, currency)}${per ? ` / ${per}` : ''}`
}

/** "Level: 125K → 250K". */
export const levelMove = (
  from: bigint,
  to: bigint,
  currency: string,
  unit?: LevelUnit
): string => `${levelSize(from, currency, unit)} → ${levelSize(to, currency, unit)}`

/** A higher level applies at once, a lower one at the end of the period (R-135). */
export const levelAppliesNow = (from: bigint, to: bigint): boolean => to > from
