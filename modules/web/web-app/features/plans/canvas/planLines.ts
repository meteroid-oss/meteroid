import Decimal from 'decimal.js'

import { metricUnitFromProto } from '@/features/billing/metrics/units'
import { TERMS, optionCell, termAdverb, unpricedIn } from '@/features/plans/currencies/priceCells'
import { fixedQuantityTotal } from '@/features/pricing/fixedQuantity'

import { inArrears, money, seatUnit } from './billingWords'

import type { ChargeContext } from './billingWords'
import type { OptionLike } from '@/features/plans/currencies/priceCells'
import type { PriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb'
import type { ListPriceComponentResponse } from '@/rpc/api/pricecomponents/v1/pricecomponents_pb'
import type { Price } from '@/rpc/api/prices/v1/models_pb'
import type { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'

// The plan editor's read-only lines, derived from the charges: where an option is not sold yet,
// and what its first invoice holds.

type Charge = Pick<PriceComponent, 'id' | 'name' | 'prices' | 'optionPrices' | 'productId'>
export type ContextOf = (component: Charge) => ChargeContext

/** Each charge's product and metric from the list, for the lines read across charges. */
export const contextFrom = (list: ListPriceComponentResponse | undefined) => {
  const products = new Map(list?.products.map(p => [p.id, p]))
  const metrics = new Map(list?.metrics.map(m => [m.id, m]))
  return (component: Pick<PriceComponent, 'productId'>): ChargeContext => {
    const product = component.productId ? products.get(component.productId) : undefined
    const structure = product?.feeStructure?.structure
    const metric =
      structure?.case === 'usage' || structure?.case === 'capacity'
        ? metrics.get(structure.value.metricId)
        : undefined
    return { product, metric, unit: metricUnitFromProto(metric) }
  }
}

const names = (charges: Pick<Charge, 'name'>[]) => {
  const all = charges.map(c => c.name)
  return all.length < 2 ? (all[0] ?? '') : `${all.slice(0, -1).join(', ')} and ${all[all.length - 1]}`
}

/** A currency, or one option in it, that a priced charge has no price for. */
export type SellingGap = { currency: string; option?: OptionLike; charges: Charge[] }

/**
 * Where the plan is not sold yet, currency by currency: the options lacking a price for a
 * priced charge, usage included. The same charges lacking in every option are one gap.
 */
export function sellingGaps(
  components: Charge[],
  options: OptionLike[],
  currencies: string[]
): SellingGap[] {
  return currencies.flatMap(currency => {
    const gaps = options
      .map(option => ({ option, charges: unpricedIn(components, option.id, currency) }))
      .filter(gap => gap.charges.length > 0)
    if (!gaps.length) return []
    const everywhere =
      gaps.length === options.length &&
      gaps.every(gap => names(gap.charges) === names(gaps[0].charges))
    if (everywhere || options.length < 2) return [{ currency, charges: gaps[0].charges }]
    return gaps.map(gap => ({ currency, option: gap.option, charges: gap.charges }))
  })
}

/** A gap in three parts, the charges set apart: "EUR, Annual" · "Seats" · what it blocks. */
export function gapParts(gap: SellingGap, planName: string) {
  const one = gap.charges.length === 1
  const until = one ? 'until it has one' : 'until each has one'
  return {
    where: gap.option ? `${gap.currency}, ${gap.option.name}` : gap.currency,
    charges: names(gap.charges),
    blocks: gap.option
      ? `${planName} won't be sold as ${gap.option.name} in ${gap.currency} ${until}.`
      : `${planName} won't be sold in ${gap.currency} ${until}.`,
  }
}

/** "EUR, Annual has no price for Seats: Pro won't be sold as Annual in EUR until it has one." */
export const gapSentence = (gap: SellingGap, planName: string) => {
  const parts = gapParts(gap, planName)
  return `${parts.where} has no price for ${parts.charges}: ${parts.blocks}`
}

/** What a price bills at the start: an amount, plus what it can't know ("seats"); none: later. */
const upfront = (
  price: Price,
  ctx: ChargeContext
): { amount: Decimal; unknown?: string } | undefined => {
  const { pricing } = price
  switch (pricing.case) {
    case 'oneTimePricing':
      return {
        amount: fixedQuantityTotal(
          Math.max(pricing.value.quantity, 1),
          pricing.value.unitPrice,
          pricing.value.model
        ),
      }
    case 'extraRecurringPricing':
      return inArrears(price, ctx)
        ? undefined
        : {
            amount: fixedQuantityTotal(
              Math.max(pricing.value.quantity, 1),
              pricing.value.unitPrice,
              pricing.value.model
            ),
          }
    case 'ratePricing':
      return { amount: new Decimal(pricing.value.rate || 0) }
    case 'capacityPricing':
      return { amount: new Decimal(pricing.value.rate || 0) }
    case 'creditPricing':
      return {
        amount: new Decimal(pricing.value.unitPrice || 0).mul(Math.max(pricing.value.quantity, 1)),
      }
    case 'slotPricing': {
      const min = pricing.value.minSlots ?? 0
      if (!min) return { amount: new Decimal(0), unknown: seatUnit(ctx).plural ?? 'seats' }
      const { model, unitRate } = pricing.value
      return {
        amount: model
          ? fixedQuantityTotal(min, unitRate, model)
          : new Decimal(unitRate || 0).mul(min),
      }
    }
    default:
      return undefined
  }
}

const order = (cadence: BillingPeriod) => TERMS.findIndex(t => t.term === cadence)

/** "usage invoiced monthly": what bills after its period, on the shortest such cadence. */
const laterWords = (cells: { price: Price; ctx: ChargeContext }[]) => {
  const later = cells
    .filter(({ price, ctx }) => inArrears(price, ctx) || price.pricing.case === 'capacityPricing')
    .map(({ price }) => ({ usage: price.pricing.case !== 'extraRecurringPricing', cadence: price.cadence }))
  if (!later.length) return undefined
  const usage = later.some(l => l.usage)
  const fees = later.some(l => !l.usage)
  const what = usage && fees ? 'usage and fees in arrears' : usage ? 'usage' : 'fees in arrears'
  const shortest = later.map(l => l.cadence).sort((a, b) => order(a) - order(b))[0]
  return `${what} invoiced ${termAdverb(shortest)}`
}

/**
 * An option's first invoice in `currency`: one-time fees and the recurring fees billed upfront
 * ("$99 at start"), then what bills later. None when a fee billed upfront has no price there.
 */
export function firstInvoice(
  components: Charge[],
  optionId: string,
  currency: string,
  contextOf: ContextOf
): { head: string; later?: string } | undefined {
  const priced = components.filter(c => c.prices.length > 0)
  if (!priced.length) return undefined
  const fee = money(currency)

  let total = new Decimal(0)
  const unknown = new Set<string>()
  const cells: { price: Price; ctx: ChargeContext }[] = []
  for (const component of priced) {
    const ctx = contextOf(component)
    const price = optionCell(component, optionId, currency)[0]
    if (!price) {
      // Usage lacking a price leaves the first invoice as is; an upfront fee leaves it unknown.
      if (upfront(component.prices[0], ctx)) return undefined
      continue
    }
    cells.push({ price, ctx })
    const due = upfront(price, ctx)
    if (!due) continue
    total = total.add(due.amount)
    if (due.unknown) unknown.add(due.unknown)
  }

  const head =
    total.isZero() && !unknown.size
      ? 'nothing at start'
      : `${fee(total)}${[...unknown].map(u => ` + ${u}`).join('')} at start`
  return { head, later: laterWords(cells) }
}

/** "First invoice: $17,000 at start · usage invoiced monthly", for one option. */
export function firstInvoiceLine(
  components: Charge[],
  optionId: string,
  currency: string,
  contextOf: ContextOf
): string | undefined {
  const invoice = firstInvoice(components, optionId, currency, contextOf)
  return invoice && [`First invoice: ${invoice.head}`, invoice.later].filter(Boolean).join(' · ')
}
