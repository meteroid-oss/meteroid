import { create } from '@bufbuild/protobuf'
import Decimal from 'decimal.js'

import { formatMetricQuantity, formatPerUnit, unitPlural } from '@/features/billing/metrics/units'
import { formatPrice } from '@/features/plans/pricecomponents/utils'
import { FIXED_MODEL_NAMES, fixedQuantityTotal } from '@/features/pricing/fixedQuantity'
import { cardRatesLabelOfMultiplier } from '@/features/productCatalog/ratecards/markup'
import { PriceInputSchema } from '@/rpc/api/pricecomponents/v1/models_pb'
import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'

import type { ChargeContext } from './billingWords'
import type { PriceInput } from '@/rpc/api/pricecomponents/v1/models_pb'
import type { Price } from '@/rpc/api/prices/v1/models_pb'

export const CADENCE_SUFFIX: Record<number, string> = {
  [BillingPeriod.MONTHLY]: '/ mo',
  [BillingPeriod.QUARTERLY]: '/ qtr',
  [BillingPeriod.SEMIANNUAL]: '/ 6 mo',
  [BillingPeriod.ANNUAL]: '/ yr',
}

const PERIODIC = ['ratePricing', 'slotPricing', 'capacityPricing', 'extraRecurringPricing']

/** Billed every period: its cadence is a choice, unlike usage's or a one-time fee's. */
export const isPeriodic = (price: Pick<Price, 'pricing'>) =>
  PERIODIC.includes(price.pricing.case ?? '')

/** The one amount a simple price holds, edited in place; none for a model (tiers, rate card…). */
export const simpleAmountOf = (price: Pick<Price, 'pricing'>): string | undefined => {
  const { pricing } = price
  switch (pricing.case) {
    case 'ratePricing':
      return pricing.value.rate
    case 'slotPricing':
      return pricing.value.model ? undefined : pricing.value.unitRate
    case 'usagePricing':
      return pricing.value.model.case === 'perUnit' ? pricing.value.model.value : undefined
    case 'extraRecurringPricing':
    case 'oneTimePricing':
      return pricing.value.model ? undefined : pricing.value.unitPrice
    default:
      return undefined
  }
}

const withAmount = (price: Pick<Price, 'pricing'>, amount: string): PriceInput['pricing'] => {
  const { pricing } = price
  switch (pricing.case) {
    case 'ratePricing':
      return { case: 'ratePricing', value: { ...pricing.value, rate: amount } }
    case 'slotPricing':
      return { case: 'slotPricing', value: { ...pricing.value, unitRate: amount } }
    case 'usagePricing':
      return {
        case: 'usagePricing',
        value: { ...pricing.value, model: { case: 'perUnit', value: amount } },
      }
    case 'extraRecurringPricing':
      return { case: 'extraRecurringPricing', value: { ...pricing.value, unitPrice: amount } }
    case 'oneTimePricing':
      return { case: 'oneTimePricing', value: { ...pricing.value, unitPrice: amount } }
    default:
      return { case: undefined }
  }
}

/** `template`'s pricing at `amount`, on `cadence`, in `currency`. */
export const priceInputOf = (
  template: Pick<Price, 'pricing' | 'creditUnit'>,
  amount: string,
  cadence: BillingPeriod,
  currency: string
): PriceInput =>
  create(PriceInputSchema, {
    cadence,
    currency,
    pricing: withAmount(template, amount),
    creditUnit: template.creditUnit,
  })

export const isAmount = (value: string) => /^\d+(\.\d+)?$/.test(value.trim())

const slotUnit = (ctx: ChargeContext) => {
  const structure = ctx.product?.feeStructure?.structure
  return (structure?.case === 'slot' && structure.value.unitName) || 'seat'
}

const MODEL_NAMES: Record<string, string> = {
  tiered: 'tiered',
  volume: 'volume',
  package: 'package',
  matrix: 'by dimension',
  rateCard: 'rate card',
}

/** A price as its cell shows it: the amount, then how often and per what. */
export const priceText = (
  price: Price,
  ctx: ChargeContext
): { main: string; suffix?: string } => {
  const money = price.creditUnit
    ? (amount: string) => `${new Decimal(amount || 0).toString()} credits`
    : formatPrice(price.currency)
  const every = CADENCE_SUFFIX[price.cadence] ?? ''
  const { pricing } = price
  switch (pricing.case) {
    case 'ratePricing':
      return { main: money(pricing.value.rate), suffix: every }
    case 'slotPricing':
      return pricing.value.model
        ? {
            main: `${MODEL_NAMES[pricing.value.model.model.case ?? ''] ?? 'Priced'} ${slotUnit(ctx)}s`,
            suffix: every,
          }
        : { main: money(pricing.value.unitRate), suffix: `/ ${slotUnit(ctx)} ${every}` }
    case 'capacityPricing':
      return {
        main: money(pricing.value.rate),
        suffix: `${every}, overage ${money(pricing.value.overageRate)}`,
      }
    case 'usagePricing': {
      const model = pricing.value.model
      switch (model.case) {
        case 'perUnit':
          return { main: money(model.value), suffix: formatPerUnit(ctx.unit) }
        case 'tiered':
        case 'volume': {
          const [first, second] = [...model.value.rows].sort(
            (a, b) => Number(a.firstUnit) - Number(b.firstUnit)
          )
          // A free first tier reads as an allowance: "1M free, then $0.0004".
          if (
            model.case === 'tiered' &&
            second &&
            new Decimal(first.unitPrice || 0).isZero() &&
            !first.flatFee
          ) {
            return {
              main: `${formatMetricQuantity(Number(second.firstUnit), ctx.unit, { compact: true }).trim()} free, then ${money(second.unitPrice)}`,
              suffix: formatPerUnit(ctx.unit),
            }
          }
          const lowest = model.value.rows
            .map(row => new Decimal(row.unitPrice || 0))
            .reduce((min, p) => (p.lessThan(min) ? p : min), new Decimal(Infinity))
          return lowest.isFinite()
            ? {
                main: `${MODEL_NAMES[model.case]}, from ${money(lowest.toString())}`,
                suffix: formatPerUnit(ctx.unit),
              }
            : { main: MODEL_NAMES[model.case] }
        }
        case 'package':
          return {
            main: money(model.value.packagePrice),
            suffix: `per ${model.value.blockSize} ${unitPlural(ctx.unit)}`,
          }
        case 'dimensionRates':
          return { main: 'By dimension', suffix: `${model.value.rates.length} rates` }
        case 'percentage':
          return { main: `${model.value.percent}%`, suffix: 'of the amount' }
        case 'dynamic':
          return { main: 'Price from events' }
        case 'rateCard':
          return {
            main: 'By dimension, rate card',
            suffix: model.value.multiplier ? cardRatesLabelOfMultiplier(model.value.multiplier) : undefined,
          }
        default:
          return { main: 'Usage' }
      }
    }
    case 'extraRecurringPricing':
    case 'oneTimePricing': {
      const { unitPrice, quantity, model } = pricing.value
      const once = pricing.case === 'oneTimePricing' ? 'once' : every
      if (!model) return { main: money(unitPrice), suffix: once }
      const total = fixedQuantityTotal(Math.max(quantity, 1), unitPrice, model).toString()
      const how = FIXED_MODEL_NAMES[model.model.case ?? ''] ?? 'by quantity'
      return { main: money(total), suffix: `${once} · ${how}` }
    }
    case 'creditPricing':
      return {
        main: money(pricing.value.unitPrice),
        suffix: pricing.value.once ? 'once' : every,
      }
    default:
      return { main: 'Priced' }
  }
}

/** The price a new one copies the structure of: the closest in currency, then cadence. */
export const templateFor = (
  prices: Price[],
  currency: string,
  cadence: BillingPeriod | undefined
): Price | undefined =>
  prices.find(p => p.currency === currency && p.cadence === cadence) ??
  prices.find(p => p.currency === currency) ??
  prices.find(p => p.cadence === cadence) ??
  prices[0]
