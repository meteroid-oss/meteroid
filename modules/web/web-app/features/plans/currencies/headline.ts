import Decimal from 'decimal.js'

import { money } from '@/features/plans/canvas/billingWords'
import { orderCurrencies, termPer } from '@/features/plans/currencies/priceCells'

import type { PricedOption } from '@/rpc/api/plans/v1/models_pb'

type Headlined = Pick<
  PricedOption,
  'currency' | 'optionId' | 'amount' | 'period' | 'hasUsage' | 'fromLowest'
> &
  Partial<Pick<PricedOption, 'unit'>>

/** "€29 / month", "€14 / seat / month", "from €49 / month", "Free", "Usage-based"; none when no
 * amount reads as its price (a charge the server can't read). */
export const priceLabel = (priced: Headlined): string | undefined => {
  if (priced.amount === undefined) return priced.hasUsage ? 'Usage-based' : undefined
  if (new Decimal(priced.amount || 0).isZero() && !priced.hasUsage) return 'Free'
  const per = priced.period === undefined ? '' : ` / ${termPer(priced.period)}`
  const unit = priced.unit ? ` / ${priced.unit}` : ''
  return `${priced.fromLowest ? 'from ' : ''}${money(priced.currency)(priced.amount)}${unit}${per}`
}

export type Headline<P extends Headlined = Headlined> = {
  /** The default option's price in the currency shown. */
  lead: P
  /** The other options sold in that currency, in option order. */
  others: P[]
  /** Every currency it is sold in, the one shown first. */
  currencies: string[]
}

/** A plan's price in `preferred` (the tenant's) when it is sold there, else in the first
 * currency it is. */
export function headlineOf<P extends Headlined>(
  priced: P[],
  preferred: string | undefined
): Headline<P> | undefined {
  const currency = priced.find(p => p.currency === preferred)?.currency ?? priced[0]?.currency
  if (currency === undefined) return undefined
  const [lead, ...others] = priced.filter(p => p.currency === currency)
  return {
    lead,
    others,
    currencies: orderCurrencies(priced.map(p => p.currency), currency),
  }
}
