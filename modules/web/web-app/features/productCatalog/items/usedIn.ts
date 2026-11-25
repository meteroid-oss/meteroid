import { rowWords } from '@/features/plans/canvas/billingWords'
import { optionCell, sameInEveryOption } from '@/features/plans/currencies/priceCells'

import type { ChargeContext } from '@/features/plans/canvas/billingWords'
import type { OptionLike } from '@/features/plans/currencies/priceCells'
import type { PriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb'
import type { Price } from '@/rpc/api/prices/v1/models_pb'

/** One cell of a charge: its prices for an option (none named: every option's), empty a gap. */
export type UsedInCell = { option?: string; prices: Price[] }

/** A charge's cells in one currency. */
export type UsedInLine = { currency: string; cells: UsedInCell[] }

type Charge = Pick<PriceComponent, 'prices' | 'optionPrices'>

/** The currencies to show: the version's, then any only the charge has, sorted. */
const currenciesOf = (charge: Charge, currencies: string[]) =>
  [...new Set([...currencies, ...charge.prices.map(p => p.currency)])].sort()

/**
 * A charge's prices by currency, then option: one cell when every option shares them (or there
 * is one option), a cell per option otherwise. A currency it has no price in is one gap.
 */
export function chargeLines(
  charge: Charge,
  options: Pick<OptionLike, 'id' | 'name'>[],
  currencies: string[]
): UsedInLine[] {
  const cell = (optionId: string | undefined, currency: string) =>
    optionId === undefined
      ? charge.prices.filter(p => p.currency === currency)
      : optionCell(charge, optionId, currency)
  return currenciesOf(charge, currencies).map(currency => {
    if (!charge.prices.some(p => p.currency === currency)) {
      return { currency, cells: [{ prices: [] }] }
    }
    if (options.length < 2 || sameInEveryOption(charge, options, currency)) {
      return { currency, cells: [{ prices: cell(options[0]?.id, currency) }] }
    }
    return {
      currency,
      cells: options.map(option => ({ option: option.name, prices: cell(option.id, currency) })),
    }
  })
}

/** The lines as text, to tell whether two versions price a charge alike. */
export const linesKey = (lines: UsedInLine[], ctx: ChargeContext) =>
  lines
    .map(
      line =>
        `${line.currency}:${line.cells
          .map(c => `${c.option ?? ''}=${rowWords(c.prices, line.currency, ctx)?.amount ?? '-'}`)
          .join(',')}`
    )
    .join(';')

/** The gap's chip: "Annual: no price", or "No price" for the whole currency. */
export const gapLabel = (cell: UsedInCell) =>
  cell.option ? `${cell.option}: no price` : 'No price'
