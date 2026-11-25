import { billedWords, countsWords } from '@/features/plans/canvas/billingWords'
import { TERMS, hasTerm, resolveCell } from '@/features/plans/currencies/priceCells'

import type { OptionLike } from '@/features/plans/currencies/priceCells'
import type { AddOn } from '@/rpc/api/addons/v1/models_pb'
import type { Price } from '@/rpc/api/prices/v1/models_pb'
import type { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'

type Priced = Pick<Price, 'pricing' | 'cadence' | 'currency' | 'creditUnit'>

/** Every price of the add-on; one listed before `prices` existed has its first only. */
export const addOnPrices = (addOn: Pick<AddOn, 'prices' | 'price'>): Price[] =>
  addOn.prices.length ? addOn.prices : addOn.price ? [addOn.price] : []

/**
 * The add-on in one line: "Flat fee · billed monthly or yearly, upfront · one per subscription ·
 * customers can add it themselves".
 */
export const addOnLine = (
  addOn: Pick<AddOn, 'prices' | 'price' | 'feeStructure' | 'maxInstancesPerSubscription' | 'selfServiceable'>
): string => {
  const prices: Priced[] = addOnPrices(addOn)
  const first = prices[0]
  const ctx = { product: addOn.feeStructure ? { feeStructure: addOn.feeStructure } : undefined }
  const cadences = TERMS.map(t => t.term).filter(term =>
    prices.some(p => hasTerm(p) && p.cadence === term)
  )
  const max = addOn.maxInstancesPerSubscription
  return [
    countsWords(first, ctx),
    first && billedWords(first, ctx, cadences.length ? cadences : undefined),
    max === 1 ? 'one per subscription' : max ? `up to ${max} per subscription` : 'any number per subscription',
    addOn.selfServiceable ? 'customers can add it themselves' : 'added by your team',
  ]
    .filter(Boolean)
    .join(' · ')
}

type Sold = { currency: string; optionId: string; period?: BillingPeriod }

export type OfferChip = { sold: boolean; label: string }

/**
 * Where a plan is sold, whether a subscription is offered the add-on: one chip per currency for
 * the options it is sold with ("USD Monthly · Annual"), one per gap ("EUR Annual: no price").
 */
export function offerChips(
  prices: Priced[],
  sold: Sold[],
  options: Pick<OptionLike, 'id' | 'name'>[]
): OfferChip[] {
  const currencies = [...new Set(sold.map(s => s.currency))]
  const named = options.length > 1
  const nameOf = (id: string) => options.find(o => o.id === id)?.name ?? ''
  return currencies.flatMap(currency => {
    const here = sold.filter(s => s.currency === currency)
    // As the server resolves it: its price on the option's cadence, or its single cadence.
    const offered = (s: Sold) =>
      s.period === undefined
        ? prices.length === 0 || prices.some(p => p.currency === currency)
        : resolveCell({ prices }, currency, s.period).kind !== 'missing'
    const yes = here.filter(offered)
    const no = here.filter(s => !offered(s))
    const names = (list: Sold[]) => `${currency} ${list.map(s => nameOf(s.optionId)).join(' · ')}`
    return [
      ...(yes.length ? [{ sold: true, label: named ? names(yes) : currency }] : []),
      ...(no.length
        ? [{ sold: false, label: `${named && yes.length ? names(no) : currency}: no price` }]
        : []),
    ]
  })
}
