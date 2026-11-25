import { resolveCell } from '@/features/plans/currencies/priceCells'

import type { AddOn } from '@/rpc/api/addons/v1/models_pb'
import type { Price } from '@/rpc/api/prices/v1/models_pb'
import type { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'

/** Every price of an add-on; an older server sends its one `price` only. */
export const addOnPrices = (addOn: Pick<AddOn, 'price' | 'prices'>): Price[] =>
  addOn.prices.length ? addOn.prices : addOn.price ? [addOn.price] : []

/**
 * The price a subscription in `currency` buys the add-on at: on `period`, or on its one cadence
 * when it has a single one (as billing reads it). None: not offered in that currency.
 */
export function addOnPriceIn(
  addOn: Pick<AddOn, 'price' | 'prices'>,
  currency: string | undefined,
  period?: BillingPeriod
): Price | undefined {
  const prices = addOnPrices(addOn)
  if (!currency) return addOn.price ?? prices[0]
  if (period === undefined) {
    const inCurrency = prices.filter(p => p.currency === currency)
    return inCurrency.find(p => p.id === addOn.price?.id) ?? inCurrency[0]
  }
  const cell = resolveCell({ prices }, currency, period)
  return cell.kind === 'own' ? cell.prices[0] : undefined
}
