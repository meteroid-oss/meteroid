import { create } from '@bufbuild/protobuf'
import Decimal from 'decimal.js'
import { useMemo } from 'react'

import { priceWords, seatUnit } from '@/features/plans/canvas/billingWords'
import { useQuery } from '@/lib/connectrpc'
import { formatCurrencyNoRounding } from '@/lib/utils/numbers'
import { previewPrice } from '@/rpc/api/prices/v1/prices-PricesService_connectquery'
import { PreviewPriceItemSchema } from '@/rpc/api/prices/v1/prices_pb'

import type { ChargeContext } from '@/features/plans/canvas/billingWords'
import type { Price, UsagePricing } from '@/rpc/api/prices/v1/models_pb'

// What a subscription or quote line bills: its count, one unit's price and the amount. A count
// priced by a model (seat tiers, a fixed quantity in packages) has no single rate: its price
// reads in words and its amount comes from the server, which prices it as billing does.

export interface ChargeFigures {
  /** The count billed: the fixed quantity, or the seats once set. */
  quantity?: number
  /** One unit's price, or the price in words when a model prices the count. */
  unitPrice: string
  /** `unitPrice` is a sentence, not one amount. */
  inWords: boolean
  /** Each period's amount; none until the seats are set or the server prices the model. */
  amount?: string
  /** The model the server prices at `quantity` for the amount. */
  model?: UsagePricing
}

const money = (amount: Decimal.Value, currency: string) =>
  formatCurrencyNoRounding(new Decimal(amount || 0).toString(), currency)

/** A fixed count's model in words: "first 10 at $5, then $4 per unit". */
const modelWords = (price: Price, model: UsagePricing) =>
  priceWords({
    pricing: { case: 'usagePricing', value: model },
    cadence: price.cadence,
    currency: price.currency,
    creditUnit: undefined,
  })

/**
 * The figures of a flat fee, seats or a fixed quantity (recurring or one-time); undefined for
 * other prices (usage, capacity, credit packs), which read as their own summary.
 */
export function quantityFigures(
  price: Price,
  { seats, ctx = {} }: { seats?: number; ctx?: ChargeContext } = {}
): ChargeFigures | undefined {
  const { currency, pricing } = price
  switch (pricing.case) {
    case 'ratePricing': {
      const rate = money(pricing.value.rate, currency)
      return { unitPrice: rate, inWords: false, amount: rate }
    }
    case 'slotPricing': {
      const { unitRate, model, minSlots, maxSlots } = pricing.value
      // The seats billed: the count set, within the price's minimum and maximum.
      const billed =
        seats === undefined
          ? undefined
          : Math.min(Math.max(seats, minSlots ?? 0), maxSlots ?? Number.POSITIVE_INFINITY)
      if (model?.model.case) {
        return {
          quantity: billed,
          unitPrice: priceWords(price, ctx),
          inWords: true,
          model: billed !== undefined ? model : undefined,
        }
      }
      return {
        quantity: billed,
        unitPrice: `${money(unitRate, currency)} per ${seatUnit(ctx).singular}`,
        inWords: false,
        amount: billed !== undefined ? money(new Decimal(unitRate || 0).mul(billed), currency) : undefined,
      }
    }
    case 'extraRecurringPricing':
    case 'oneTimePricing': {
      const { unitPrice, model } = pricing.value
      const quantity = Math.max(pricing.value.quantity, 1)
      if (model?.model.case) {
        return { quantity, unitPrice: modelWords(price, model), inWords: true, model }
      }
      return {
        quantity,
        unitPrice: money(unitPrice, currency),
        inWords: false,
        amount: money(new Decimal(unitPrice || 0).mul(quantity), currency),
      }
    }
    default:
      return undefined
  }
}

export interface ModelAmountRequest {
  key: string
  model: UsagePricing
  quantity: number
}

/** The figures' requests for the server, under `key`. */
export const modelAmountRequest = (
  key: string,
  figures: ChargeFigures | undefined
): ModelAmountRequest[] =>
  figures?.model && figures.quantity !== undefined
    ? [{ key, model: figures.model, quantity: figures.quantity }]
    : []

const bigintSafe = (_: string, v: unknown) => (typeof v === 'bigint' ? v.toString() : v)

/** What the server bills each model at its quantity, formatted, by key; none while it prices. */
export function useModelAmounts(
  requests: ModelAmountRequest[],
  currency: string
): Map<string, string | undefined> {
  const items = useMemo(
    () =>
      requests.map(r =>
        create(PreviewPriceItemSchema, {
          key: r.key,
          usagePricing: r.model,
          quantity: String(r.quantity),
          currency,
        })
      ),
    [requests, currency]
  )
  const hash = JSON.stringify(items, bigintSafe)
  const query = useQuery(
    previewPrice,
    { items },
    {
      enabled: items.length > 0,
      // uint64 fields are BigInt, which the default key hashing cannot serialize.
      queryKeyHashFn: () => `previewPrice-${hash}`,
    }
  )
  return useMemo(
    () =>
      new Map(
        (query.data?.results ?? []).map(r => [
          r.key,
          r.amount !== undefined ? money(r.amount, currency) : undefined,
        ])
      ),
    [query.data, currency]
  )
}

/** The figures with the server's amount filled in where a model prices them. */
export const withModelAmount = (
  figures: ChargeFigures | undefined,
  amounts: Map<string, string | undefined>,
  key: string
): ChargeFigures | undefined =>
  figures?.model ? { ...figures, amount: amounts.get(key) } : figures
