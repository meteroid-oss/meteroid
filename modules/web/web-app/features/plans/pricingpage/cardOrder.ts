import { extraLabel } from '@/features/plans/pricingpage/cardPrice'
import {
  CardExtra_Kind,
  type PricingCard,
  type PricingCharge,
  PricingCharge_Kind,
} from '@/rpc/api/pricingpage/v1/pricingpage_pb'

// The order a plan's card reads its charges in, as its editor lists it: one row per item of the
// card (the base price groups the fixed fees on the card's period), the first that can lead being
// the card's price. Saved by product, kept across versions.

/** A row of the card's order: an item of the card, or a charge left off it. */
export interface OrderRow {
  /** Its products, joined: what a drag moves. */
  id: string
  productIds: string[]
  name: string
  /** How it reads on the card, in the currency shown: "$12 / seat (3 included)". */
  reading: string
  /** Any row but the card's price can be left off it. */
  hideable: boolean
  hidden: boolean
  /** The card's price. */
  lead: boolean
}

const KIND_READING: Record<PricingCharge_Kind, string> = {
  [PricingCharge_Kind.FIXED]: 'Fixed fee',
  [PricingCharge_Kind.SEATS]: 'Per seat',
  [PricingCharge_Kind.USAGE]: 'Usage',
  [PricingCharge_Kind.ONE_TIME]: 'One-time fee',
  [PricingCharge_Kind.CREDIT_PACK]: 'Credit pack',
}

/** The rows of `card` (the server's reading with the draft's order), in its order; without one
 * (the plan is not on the page), the plan's charges by kind. */
export const orderRows = (
  card: Pick<PricingCard, 'price'> | undefined,
  charges: PricingCharge[],
  currency: string,
  hidden: string[],
  locale?: string
): OrderRow[] => {
  const charge = (id: string | undefined) => charges.find(c => c.productId === id)
  const price = card?.price
  if (!price) {
    return charges.map(c => ({
      id: c.productId,
      productIds: [c.productId],
      name: c.name,
      reading: KIND_READING[c.kind],
      hideable: c.canHide,
      hidden: hidden.includes(c.productId),
      lead: false,
    }))
  }
  const rows = price.extras.map(
    (item, index): OrderRow => ({
      id: item.productIds.join(',') || `item-${index}`,
      productIds: item.productIds,
      name:
        item.kind === CardExtra_Kind.BASE
          ? 'Base price'
          : item.name || charge(item.productIds[0])?.name || 'Charge',
      reading: extraLabel(item, currency, locale, price.period).replace(/^\+ /, ''),
      hideable: !item.lead && item.productIds.length > 0,
      hidden: false,
      lead: item.lead,
    })
  )
  // Left off, a charge is not on the card: its row stays, to be put back.
  const shown = new Set(price.extras.flatMap(item => item.productIds))
  const left = hidden
    .filter(id => !shown.has(id))
    .flatMap((id): OrderRow[] => {
      const known = charge(id)
      return known
        ? [
            {
              id,
              productIds: [id],
              name: known.name,
              reading: 'Not on the card',
              hideable: true,
              hidden: true,
              lead: false,
            },
          ]
        : []
    })
  return [...rows, ...left]
}

/** `rows` in the draft's `order`: those it places by their earliest product, the others after
 * as the card reads them. The list follows a drag at once, the readings once the card is read
 * again. */
export const inOrder = (rows: OrderRow[], order: string[]): OrderRow[] => {
  const place = (row: OrderRow) => {
    const at = row.productIds.map(id => order.indexOf(id)).filter(index => index >= 0)
    return at.length > 0 ? Math.min(...at) : Number.POSITIVE_INFINITY
  }
  return rows
    .map((row, index) => ({ row, index, place: place(row) }))
    .sort((a, b) => (a.place === b.place ? a.index - b.index : a.place - b.place))
    .map(({ row }) => row)
}

/** The order saved once row `id` moves to `target`'s place: every row's products in the new order,
 * then those of `order` the card doesn't show (another option's), as they were. */
export const movedOrder = (
  rows: OrderRow[],
  order: string[],
  id: string,
  target: string
): string[] => {
  const from = rows.findIndex(row => row.id === id)
  const to = rows.findIndex(row => row.id === target)
  if (from < 0 || to < 0 || from === to) return order
  const moved = [...rows]
  moved.splice(to, 0, ...moved.splice(from, 1))
  const shown = moved.flatMap(row => row.productIds)
  return [...shown, ...order.filter(productId => !shown.includes(productId))]
}
