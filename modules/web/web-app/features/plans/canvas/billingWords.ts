import Decimal from 'decimal.js'

import { formatPerUnit, rawQuantity, unitNoun, unitPlural } from '@/features/billing/metrics/units'
import { TERMS, hasTerm, termAdverb, termPer } from '@/features/plans/currencies/priceCells'
import { fixedQuantityTotal } from '@/features/pricing/fixedQuantity'
import { cardRatesLabelOfMultiplier } from '@/features/productCatalog/ratecards/markup'
import { FeeStructure_BillingType } from '@/rpc/api/prices/v1/models_pb'

import type { MetricUnit } from '@/features/billing/metrics/units'
import type { BillableMetric } from '@/rpc/api/billablemetrics/v1/models_pb'
import type { Price, UsagePricing } from '@/rpc/api/prices/v1/models_pb'
import type { Product } from '@/rpc/api/products/v1/models_pb'
import type { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'

// A charge's price in finance words: "$99 / month", "first 1M free, then $0.0004 per unit",
// "billed yearly, upfront". Pure, so the plan editor's lines are unit tested.

// Only what the words read, so a catalog list row (a ProductMeta) can say them too.
export type ChargeContext = {
  product?: Pick<Product, 'feeStructure'>
  metric?: Pick<BillableMetric, 'name'>
  unit?: MetricUnit
}

type Priced = Pick<Price, 'pricing' | 'cadence' | 'currency' | 'creditUnit'>
type TierRow = { firstUnit: bigint; unitPrice: string; flatFee?: string }
type Money = (amount: string | Decimal) => string

const compactFormat = new Intl.NumberFormat('en-US', { notation: 'compact', maximumFractionDigits: 1 })
const countFormat = new Intl.NumberFormat('en-US', { maximumFractionDigits: 6 })

/** An amount in `currency`, cents only when it has some: "$99", "$0.0004", "€12.50". */
export const money =
  (currency: string): Money =>
  amount => {
    const value = new Decimal(amount || 0)
    const fraction = value.isInteger() ? 0 : 2
    try {
      return value.toNumber().toLocaleString('en-US', {
        style: 'currency',
        currency,
        minimumFractionDigits: fraction,
        maximumFractionDigits: 8,
      })
    } catch {
      // A credit unit code is no ISO currency: shown after the amount.
      return `${countFormat.format(value.toNumber())} ${currency}`
    }
  }

const credits: Money = amount => `${countFormat.format(new Decimal(amount || 0).toNumber())} credits`

/** The fee's money, and usage's: credits when the price is in a credit unit. */
const formatters = (price: Priced) => {
  const fee = money(price.currency)
  return { fee, usageRate: price.creditUnit ? credits : fee }
}

/** A quantity in priced units as raw metric units, compact: "1M", "10K", "500". */
const compact = (priced: bigint | number, unit?: MetricUnit) =>
  compactFormat.format(rawQuantity(priced, unit) ?? Number(priced))

const isZero = (amount?: string) => new Decimal(amount || 0).isZero()

const SEAT: MetricUnit = { singular: 'seat', plural: 'seats' }

/** A seat unit from its name ("Seats", "API keys", a subscription fee's "seat"). */
const seatUnitNamed = (unitName: string): MetricUnit => {
  const raw = unitName.trim()
  if (!raw) return SEAT
  // "Seats" reads "seat"; an acronym ("API keys") keeps its case.
  const name = /^[A-Z][a-z]/.test(raw) ? raw[0].toLowerCase() + raw.slice(1) : raw
  const singular = name.endsWith('s') ? name.slice(0, -1) : name
  return { singular, plural: `${singular}s` }
}

/** What a seat charge counts, as a unit: its product's name for it, "seat" by default. */
export const seatUnit = (ctx: ChargeContext): MetricUnit => {
  const structure = ctx.product?.feeStructure?.structure
  return seatUnitNamed(structure?.case === 'slot' ? structure.value.unitName : '')
}

const byFirstUnit = (rows: TierRow[]) =>
  [...rows].sort((a, b) => Number(a.firstUnit) - Number(b.firstUnit))

/** Graduated tiers spelled out: "first 1M free, next 9M at $0.0008, then $0.0005 per unit". */
const graduated = (rows: TierRow[], m: Money, per: string, unit?: MetricUnit) => {
  const sorted = byFirstUnit(rows)
  const flat = (row: TierRow) => (isZero(row.flatFee) ? '' : ` + ${m(row.flatFee!)} flat`)
  if (sorted.length === 1) return `${m(sorted[0].unitPrice)} ${per}${flat(sorted[0])}`
  return sorted
    .map((row, i) => {
      if (i === sorted.length - 1) return `then ${m(row.unitPrice)} ${per}${flat(row)}`
      const span = compact(Number(sorted[i + 1].firstUnit) - Number(row.firstUnit), unit)
      const price =
        isZero(row.unitPrice) && !flat(row) ? 'free' : `at ${m(row.unitPrice)}${flat(row)}`
      return `${i === 0 ? 'first' : 'next'} ${span} ${price}`
    })
    .join(', ')
}

/** Volume tiers: every unit at the rate its total reaches. */
const volume = (rows: TierRow[], m: Money, per: string, unit?: MetricUnit) => {
  const sorted = byFirstUnit(rows)
  const steps = sorted.map((row, i) =>
    i === 0
      ? `${m(row.unitPrice)} ${per}`
      : `${m(row.unitPrice)} from ${compact(row.firstUnit, unit)}`
  )
  return sorted.length > 1 ? `${steps.join(', ')}, on every unit` : steps[0]
}

/** "per 1M tokens", "per 10 seats": a package's block. */
const perBlock = (blockSize: bigint, unit?: MetricUnit) => {
  const raw = rawQuantity(blockSize, unit) ?? Number(blockSize)
  return raw === 1 ? `per ${unitNoun(1, unit)}` : `per ${compact(blockSize, unit)} ${unitNoun(raw, unit)}`
}

/** A usage model in words, `per` naming its unit ("per unit", "per 1M tokens", "each"). */
const modelWords = (
  model: UsagePricing['model'],
  m: Money,
  per: string,
  unit?: MetricUnit
): string => {
  switch (model.case) {
    case 'perUnit':
      return `${m(model.value)} ${per}`
    case 'tiered':
      return graduated(model.value.rows, m, per, unit)
    case 'volume':
      return volume(model.value.rows, m, per, unit)
    case 'package':
      return `${m(model.value.packagePrice)} ${perBlock(model.value.blockSize, unit)}`
    case 'dimensionRates': {
      const rates = model.value.rates.map(rate => new Decimal(rate.rate || 0))
      if (!rates.length) return 'by dimension'
      const low = Decimal.min(...rates)
      const high = Decimal.max(...rates)
      // Rates per 1M units read "per 1M tokens".
      const each =
        new Decimal(model.value.unitSize || 1).equals(1)
          ? per
          : `per ${compactFormat.format(Number(model.value.unitSize))} ${unitNoun(2, unit)}`
      return low.equals(high)
        ? `${m(low)} ${each}, by dimension`
        : `${m(low)} to ${m(high)} ${each}, by dimension`
    }
    case 'rateCard':
      return `by dimension, at its rate card's rates${model.value.multiplier ? ` (${cardRatesLabelOfMultiplier(model.value.multiplier)})` : ''}`
    default:
      return 'priced on usage'
  }
}

/**
 * A count priced through `model`, in words and without its cadence, `unitName` naming what it
 * counts: "first 3 free, then $10 per seat", "$30 per 10 seats".
 */
export const countModelWords = (model: UsagePricing, currency: string, unitName = ''): string => {
  const unit = seatUnitNamed(unitName)
  return modelWords(model.model, money(currency), `per ${unit.singular}`, unit)
}

/** A fixed quantity priced through `model`, in words: "15 units: first 10 at $10, then $8 each". */
export const fixedQuantityModelWords = (
  quantity: number,
  model: UsagePricing,
  currency: string
): string =>
  `${countFormat.format(Math.max(quantity, 1))} units: ${modelWords(model.model, money(currency), 'each')}`

/** "/ month", "/ year": how often a recurring price is due. */
const every = (cadence: BillingPeriod) => `/ ${termPer(cadence)}`

/** What a fixed quantity costs in all, and how it is priced when that's more than one number. */
const fixedQuantity = (
  value: { unitPrice: string; quantity: number; model?: UsagePricing },
  m: Money
) => {
  const quantity = Math.max(value.quantity, 1)
  const total = fixedQuantityTotal(quantity, value.unitPrice, value.model)
  const how = value.model
    ? `${countFormat.format(quantity)} units: ${modelWords(value.model.model, m, 'each')}`
    : quantity > 1
      ? `${countFormat.format(quantity)} × ${m(value.unitPrice)}`
      : undefined
  return { total, how }
}

/** One price in words: "$99 / month", "$12 per seat / month", "$1,100 once". */
export const priceWords = (price: Priced, ctx: ChargeContext = {}): string => {
  const { fee, usageRate } = formatters(price)
  const { pricing } = price
  const per = formatPerUnit(ctx.unit)
  switch (pricing.case) {
    case 'ratePricing':
      return `${fee(pricing.value.rate)} ${every(price.cadence)}`
    case 'slotPricing': {
      const seat = seatUnit(ctx)
      const each = `per ${seat.singular}`
      return pricing.value.model
        ? `${modelWords(pricing.value.model.model, fee, each, seat)} ${every(price.cadence)}`
        : `${fee(pricing.value.unitRate)} ${each} ${every(price.cadence)}`
    }
    case 'capacityPricing': {
      const included = `${compact(pricing.value.included, ctx.unit)} ${unitPlural(ctx.unit)}`
      return `${fee(pricing.value.rate)} ${every(price.cadence)} with ${included} included, then ${usageRate(pricing.value.overageRate)} ${per}`
    }
    case 'usagePricing':
      return modelWords(pricing.value.model, usageRate, per, ctx.unit)
    case 'extraRecurringPricing':
      return `${fee(fixedQuantity(pricing.value, fee).total)} ${every(price.cadence)}`
    case 'oneTimePricing':
      return `${fee(fixedQuantity(pricing.value, fee).total)} once`
    case 'creditPricing': {
      const { unitPrice, quantity, once } = pricing.value
      const when = once ? 'once' : every(price.cadence)
      return isZero(unitPrice)
        ? `included ${when}`
        : `${fee(new Decimal(unitPrice).mul(Math.max(quantity, 1)))} ${when}`
    }
    default:
      return 'priced'
  }
}

/** Billed after the period it counts: usage, a capacity's overage, a fee set to arrears. */
export const inArrears = (price: Pick<Price, 'pricing'>, ctx: ChargeContext = {}) => {
  if (price.pricing.case === 'usagePricing') return true
  const structure = ctx.product?.feeStructure?.structure
  return (
    price.pricing.case === 'extraRecurringPricing' &&
    structure?.case === 'extraRecurring' &&
    structure.value.billingType === FeeStructure_BillingType.ARREAR
  )
}

const once = (price: Pick<Price, 'pricing'>) =>
  price.pricing.case === 'oneTimePricing' ||
  (price.pricing.case === 'creditPricing' && price.pricing.value.once)

/** "monthly or yearly", "monthly, quarterly or yearly". */
export const orList = (words: string[]) =>
  words.length < 2
    ? (words[0] ?? '')
    : `${words.slice(0, -1).join(', ')} or ${words[words.length - 1]}`

/** When a price is invoiced: "billed once", "billed yearly, upfront", "billed monthly, in arrears". */
export const billedWords = (
  price: Pick<Price, 'pricing' | 'cadence'>,
  ctx: ChargeContext = {},
  cadences: BillingPeriod[] = [price.cadence]
): string => {
  if (once(price)) return 'billed once'
  const when = orList(cadences.map(termAdverb))
  if (inArrears(price, ctx)) return `billed ${when}, in arrears`
  const overage = price.pricing.case === 'capacityPricing' ? ' · overage in arrears' : ''
  return `billed ${when}, upfront${overage}`
}

/** What a charge is, under its name: "Flat fee", "Per seat, 3 minimum", "Usage of Tokens". */
export const countsWords = (price: Priced | undefined, ctx: ChargeContext = {}): string => {
  if (!price) return 'Its features come with the plan'
  const { fee } = formatters(price)
  const { pricing } = price
  switch (pricing.case) {
    case 'ratePricing':
      return 'Flat fee'
    case 'slotPricing': {
      const min = pricing.value.minSlots ? `, ${pricing.value.minSlots} minimum` : ''
      return `Per ${seatUnit(ctx).singular}${min}`
    }
    case 'capacityPricing':
      return `${ctx.metric?.name ?? 'Usage'}, with an included amount`
    case 'usagePricing':
      return `Usage${ctx.metric?.name ? ` of ${ctx.metric.name}` : ''}${price.creditUnit ? ', in credits' : ''}`
    case 'extraRecurringPricing': {
      const { how } = fixedQuantity(pricing.value, fee)
      return `Fixed fee${how ? `, ${how}` : ''}`
    }
    case 'oneTimePricing': {
      const { how } = fixedQuantity(pricing.value, fee)
      return `One-time fee${how ? `, ${how}` : ''}`
    }
    case 'creditPricing': {
      const amount = new Decimal(pricing.value.creditAmount || 0).mul(
        Math.max(pricing.value.quantity, 1)
      )
      return `Credit pack, ${countFormat.format(amount.toNumber())} credits`
    }
    default:
      return ''
  }
}

const termOrder = (cadence: BillingPeriod) => TERMS.findIndex(t => t.term === cadence)

/** A charge's prices in `currency`, one per cadence, monthly first. */
export const pricesIn = <P extends Priced>(prices: P[], currency: string): P[] => {
  const here = prices.filter(p => p.currency === currency)
  const seen = new Set<BillingPeriod>()
  return here
    .filter(p => {
      if (!hasTerm(p)) return true
      if (seen.has(p.cadence)) return false
      seen.add(p.cadence)
      return true
    })
    .sort((a, b) => termOrder(a.cadence) - termOrder(b.cadence))
}

/** A charge's row in `currency`: "$99 / month · or $950 / year", billed "monthly or yearly". */
export const rowWords = (
  prices: Priced[],
  currency: string,
  ctx: ChargeContext = {}
): { amount: string; billed: string } | undefined => {
  const shown = pricesIn(prices, currency)
  if (!shown.length) return undefined
  const levels = prices.filter(p => p.currency === currency).length - shown.length
  const amount = shown.map(p => priceWords(p, ctx)).join(' · or ')
  const cadences = shown.filter(hasTerm).map(p => p.cadence)
  return {
    amount: levels > 0 && shown.length === 1 ? `${amount} · +${levels} levels` : amount,
    billed: billedWords(shown[0], ctx, cadences.length ? cadences : undefined),
  }
}

/** A price's words as its chip shows them: up to the last amount stands out, the rest is quiet. */
export function chipParts(text: string): { strong: string; quiet: string } {
  // Several prices ("$99 / month · or $950 / year") read as one.
  if (text.includes(' · ')) return { strong: text, quiet: '' }
  const amounts = [...text.matchAll(/[^\s,]*\d[^\s,]*/g)]
  const last = amounts[amounts.length - 1]
  if (!last || last.index === undefined) return { strong: text, quiet: '' }
  const end = last.index + last[0].length
  return { strong: text.slice(0, end), quiet: text.slice(end).trim() }
}
