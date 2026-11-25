import Decimal from 'decimal.js'

import { RateCardUnmatched } from '@/rpc/api/ratecards/v1/models_pb'

import { markupToMultiplier } from './markup'

import type { DimensionRateRow } from '@/features/pricing/schemas'
import type { RateCard, RateCardEntry } from '@/rpc/api/ratecards/v1/models_pb'

// Between a price's own rates by dimension and a shared rate card: a card's rates copied into
// the price, and a price's rates saved as a new card.

/** A price's rates by dimension, as its form keeps them. */
export interface PriceRates {
  dimensions: string[]
  keyRates: DimensionRateRow[]
  unitSize: string
  onUnmatched: 'hold' | 'ignore'
}

const anyOther = (key: string[]) => key.every(pattern => pattern === '*')

/**
 * The card's rates in force as the price's own, in the price's currency (a credit card's, in
 * its unit), the markup applied: the copy bills what the price billed from the card that day.
 * Undefined when the card has no rate the price could take.
 */
export function cardRatesForPrice(
  card: Pick<RateCard, 'dimensions' | 'unitSize' | 'onUnmatched' | 'creditUnitId'>,
  rates: Pick<RateCardEntry, 'key' | 'currency' | 'rate' | 'label'>[],
  currency: string,
  markup?: string
): PriceRates | undefined {
  const multiplier = new Decimal(markupToMultiplier(markup) ?? '1')
  const keyRates = rates
    .filter(entry => entry.rate !== undefined)
    .filter(entry => (card.creditUnitId ? !entry.currency : entry.currency === currency))
    .map((entry): DimensionRateRow => ({
      key: [...entry.key],
      rate: new Decimal(entry.rate as string).mul(multiplier).toDecimalPlaces(8).toFixed(),
      ...(entry.label ? { label: entry.label } : {}),
    }))
    // The most specific rows first, "any other" last, as the card lists them.
    .sort((a, b) =>
      anyOther(a.key) !== anyOther(b.key)
        ? Number(anyOther(a.key)) - Number(anyOther(b.key))
        : a.key.join(',').localeCompare(b.key.join(','))
    )
  if (!keyRates.length) return undefined
  return {
    dimensions: [...card.dimensions],
    keyRates,
    unitSize: card.unitSize || '1',
    onUnmatched: card.onUnmatched === RateCardUnmatched.IGNORE ? 'ignore' : 'hold',
  }
}

/**
 * A price's rates as a new card's first rates: in the price's currency, or (a price in credits)
 * without one. Rows without a rate are left out; an empty pattern matches any value.
 */
export function priceRatesForCard(
  keyRates: DimensionRateRow[],
  currency: string,
  creditUnitId?: string
): { key: string[]; currency?: string; rate: string; label?: string }[] {
  return keyRates
    .filter(row => row.rate?.trim())
    .map(row => ({
      key: row.key.map(pattern => pattern.trim() || '*'),
      currency: creditUnitId ? undefined : currency,
      rate: row.rate.trim(),
      ...(row.label ? { label: row.label } : {}),
    }))
}
