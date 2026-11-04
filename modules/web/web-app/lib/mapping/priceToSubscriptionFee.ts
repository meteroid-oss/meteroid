import { create } from '@bufbuild/protobuf';
import Decimal from 'decimal.js'

import { formatCadence } from '@/lib/mapping/prices'
import { formatCurrencyNoRounding, formatUsage } from '@/lib/utils/numbers'
import { UsageFeeSchema, PriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb';
import { Price , FeeStructure_BillingType } from '@/rpc/api/prices/v1/models_pb'
import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'
import {
  SubscriptionFeeSchema,
  SubscriptionFee_CapacitySubscriptionFeeSchema,
  SubscriptionFee_CreditSubscriptionFeeSchema,
  SubscriptionFee_ExtraRecurringSubscriptionFeeSchema,
  SubscriptionFee_FeeCreditUnitSchema,
  SubscriptionFee_OneTimeSubscriptionFeeSchema,
  SubscriptionFee_RateSubscriptionFeeSchema,
  SubscriptionFee_SlotSubscriptionFeeSchema,
  SubscriptionFeeBillingPeriod,
} from '@/rpc/api/subscriptions/v1/models_pb';

import type { CreditUnit } from '@/rpc/api/creditunits/v1/models_pb'
import type { SubscriptionFee } from '@/rpc/api/subscriptions/v1/models_pb';


/**
 * Get the first (and currently only) price from a PriceComponent.
 * With the current model, each component has exactly one price.
 */
export function getPrice(component: PriceComponent): Price | undefined {
  return component.prices[0]
}

/**
 * Get the pricing type case name from a Price, mapped to a simple label.
 */
export function getPricingTypeLabel(price: Price): string {
  switch (price.pricing.case) {
    case 'ratePricing':
      return 'Rate'
    case 'slotPricing':
      return 'Slot'
    case 'capacityPricing':
      return 'Capacity'
    case 'usagePricing':
      return 'Usage'
    case 'extraRecurringPricing':
      return 'Extra Recurring'
    case 'oneTimePricing':
      return 'One-time'
    case 'creditPricing':
      return 'Credit pack'
    default:
      return 'Unknown'
  }
}

/**
 * Map BillingPeriod → SubscriptionFeeBillingPeriod
 */
export function billingPeriodToSubscriptionPeriod(
  bp: BillingPeriod
): SubscriptionFeeBillingPeriod {
  switch (bp) {
    case BillingPeriod.MONTHLY:
      return SubscriptionFeeBillingPeriod.MONTHLY
    case BillingPeriod.QUARTERLY:
      return SubscriptionFeeBillingPeriod.QUARTERLY
    case BillingPeriod.ANNUAL:
      return SubscriptionFeeBillingPeriod.YEARLY
    case BillingPeriod.SEMIANNUAL:
      return SubscriptionFeeBillingPeriod.SEMIANNUAL
    default:
      return SubscriptionFeeBillingPeriod.MONTHLY
  }
}

/**
 * Determine the SubscriptionFeeBillingPeriod for a price.
 * One-time prices return ONE_TIME, others use the price's cadence.
 */
export function priceToSubscriptionPeriod(price: Price): SubscriptionFeeBillingPeriod {
  if (price.pricing.case === 'oneTimePricing') {
    return SubscriptionFeeBillingPeriod.ONE_TIME
  }
  return billingPeriodToSubscriptionPeriod(price.cadence)
}

const feeCreditUnit = (creditUnitId: string, creditUnits: CreditUnit[] | undefined, rate?: string) => {
  const unit = creditUnits?.find(u => u.id === creditUnitId)
  return create(SubscriptionFee_FeeCreditUnitSchema, {
    creditUnitId,
    code: unit?.code ?? 'credits',
    precision: unit?.precision ?? 0,
    rate,
    name: unit?.name,
    pluralName: unit?.pluralName,
  })
}

/**
 * Convert a Price to a SubscriptionFee.
 *
 * For usage and capacity, metricId is left empty — the backend resolves
 * it from the price component's linked product during subscription creation.
 * `creditUnits` names the unit a credit-priced usage or capacity price is expressed in, and the
 * unit a credit pack grants (no rate: the pack itself is paid in money), as the server does.
 */
export function priceToSubscriptionFee(
  price: Price,
  config?: { initialSlotCount?: number; creditUnits?: CreditUnit[] }
): SubscriptionFee {
  const fee = create(SubscriptionFeeSchema)
  const pricedIn = price.creditUnit
  if (
    pricedIn &&
    (price.pricing.case === 'usagePricing' || price.pricing.case === 'capacityPricing')
  ) {
    fee.creditUnit = feeCreditUnit(pricedIn.creditUnitId, config?.creditUnits, pricedIn.rate)
  }
  if (price.pricing.case === 'creditPricing' && price.pricing.value.creditUnitId) {
    fee.creditUnit = feeCreditUnit(price.pricing.value.creditUnitId, config?.creditUnits)
  }

  switch (price.pricing.case) {
    case 'ratePricing':
      fee.fee = {
        case: 'rate',
        value: create(SubscriptionFee_RateSubscriptionFeeSchema, {
          rate: price.pricing.value.rate,
        }),
      }
      break

    case 'slotPricing':
      fee.fee = {
        case: 'slot',
        value: create(SubscriptionFee_SlotSubscriptionFeeSchema, {
          unit: 'unit',
          unitRate: price.pricing.value.unitRate,
          minSlots: price.pricing.value.minSlots,
          maxSlots: price.pricing.value.maxSlots,
          initialSlots: config?.initialSlotCount ?? 1,
        }),
      }
      break

    case 'capacityPricing': {
      const cap = price.pricing.value
      fee.fee = {
        case: 'capacity',
        value: create(SubscriptionFee_CapacitySubscriptionFeeSchema, {
          rate: cap.rate,
          included: cap.included,
          overageRate: cap.overageRate,
          metricId: '', // resolved by backend from product
        }),
      }
      break
    }

    case 'usagePricing': {
      const usage = price.pricing.value
      // UsageFee and UsagePricing share the same model types, so pass through directly
      const usageFee = create(UsageFeeSchema, { term: price.cadence, metricId: '' })
      usageFee.model = usage.model
      fee.fee = { case: 'usage', value: usageFee }
      break
    }

    case 'oneTimePricing': {
      const ot = price.pricing.value
      const total = new Decimal(ot.unitPrice || '0').mul(ot.quantity || 1).toString()
      fee.fee = {
        case: 'oneTime',
        value: create(SubscriptionFee_OneTimeSubscriptionFeeSchema, {
          rate: ot.unitPrice,
          quantity: ot.quantity || 1,
          total,
        }),
      }
      break
    }

    case 'creditPricing': {
      const pack = price.pricing.value
      const packs = pack.quantity || 1
      fee.fee = {
        case: 'credit',
        value: create(SubscriptionFee_CreditSubscriptionFeeSchema, {
          rate: pack.unitPrice,
          quantity: packs,
          total: new Decimal(pack.unitPrice || '0').mul(packs).toString(),
          creditAmount: pack.creditAmount,
          creditTotal: new Decimal(pack.creditAmount || '0').mul(packs).toString(),
          rollover: pack.rollover,
          once: pack.once,
        }),
      }
      break
    }

    case 'extraRecurringPricing': {
      const er = price.pricing.value
      const total = new Decimal(er.unitPrice || '0').mul(er.quantity || 1).toString()
      fee.fee = {
        case: 'recurring',
        value: create(SubscriptionFee_ExtraRecurringSubscriptionFeeSchema, {
          rate: er.unitPrice,
          quantity: er.quantity || 1,
          total,
          billingType: FeeStructure_BillingType.ARREAR,
        }),
      }
      break
    }
  }

  return fee
}

/**
 * Extract a simple unit price from a Price for display purposes.
 */
export function getPriceUnitPrice(price: Price): number {
  switch (price.pricing.case) {
    case 'ratePricing':
      return parseFloat(price.pricing.value.rate || '0')
    case 'slotPricing':
      return parseFloat(price.pricing.value.unitRate || '0')
    case 'capacityPricing':
      return parseFloat(price.pricing.value.rate || '0')
    case 'oneTimePricing':
      return parseFloat(price.pricing.value.unitPrice || '0')
    case 'extraRecurringPricing':
      return parseFloat(price.pricing.value.unitPrice || '0')
    case 'creditPricing':
      return parseFloat(price.pricing.value.unitPrice || '0')
    case 'usagePricing':
      return 0 // metered, no simple unit price
    default:
      return 0
  }
}

/**
 * Get component pricing info for display in subscription review.
 */
export function getComponentPricingFromPrice(
  price: Price,
  config?: { initialSlotCount?: number }
): {
  unitPrice: number
  quantity: number
  total: number
  isMetered: boolean
} {
  if (price.pricing.case === 'usagePricing') {
    return { unitPrice: 0, quantity: 1, total: 0, isMetered: true }
  }

  const unitPrice = getPriceUnitPrice(price)
  let quantity = 1

  if (price.pricing.case === 'slotPricing') {
    quantity = config?.initialSlotCount ?? 1
  } else if (price.pricing.case === 'oneTimePricing') {
    quantity = price.pricing.value.quantity || 1
  } else if (price.pricing.case === 'extraRecurringPricing') {
    quantity = price.pricing.value.quantity || 1
  } else if (price.pricing.case === 'creditPricing') {
    quantity = price.pricing.value.quantity || 1
  }

  return {
    unitPrice,
    quantity,
    total: unitPrice * quantity,
    isMetered: false,
  }
}

/** @deprecated Use `formatCadence` from `@/lib/mapping/prices` instead. */
export const getBillingPeriodLabel = formatCadence

export function getPriceBillingLabel(price: Price): string {
  if (price.pricing.case === 'oneTimePricing') {
    return 'One-time'
  }
  if (price.pricing.case === 'creditPricing' && price.pricing.value.once) {
    return 'Once at start'
  }
  return formatCadence(price.cadence)
}

// Mirrors the server's `sells_credit`: a credit pack with a non-zero price.
export const isPaidCreditPack = (price: Price | undefined): boolean => {
  if (price?.pricing.case !== 'creditPricing') return false
  try {
    return new Decimal(price.pricing.value.unitPrice || '0').greaterThan(0)
  } catch {
    return false
  }
}

/**
 * The net terms the server applies when a subscription or quote names none (store
 * `default_net_terms`): due on receipt when it sells credit, the plan's own terms otherwise.
 */
export const defaultNetTerms = (
  prices: (Price | undefined)[],
  planNetTerms: number
): { netTerms: number; sellsCredit: boolean } => {
  const sellsCredit = prices.some(isPaidCreditPack)
  return { netTerms: sellsCredit ? 0 : planNetTerms, sellsCredit }
}

export const formatNetTerms = (netTerms: number): string =>
  netTerms > 0 ? `Net ${netTerms} days` : 'Due on receipt'

export const formatDefaultNetTerms = ({
  netTerms,
  sellsCredit,
}: {
  netTerms: number
  sellsCredit: boolean
}): string => `${formatNetTerms(netTerms)} (${sellsCredit ? 'sells credit' : "plan's terms"})`

type CreditUnitNaming = Pick<CreditUnit, 'id' | 'name' | 'pluralName'>

/**
 * Format a usage-based Price for display, showing the pricing model and rate summary.
 * A price in a credit unit is shown in credits, named from `creditUnits` when given.
 */
export function formatUsagePriceSummary(
  price: Price,
  currency: string,
  creditUnits?: readonly CreditUnitNaming[]
): { model: string; amount: string; note?: string } {
  if (price.pricing.case !== 'usagePricing') {
    return { model: '', amount: '' }
  }
  const summary = usagePriceSummary(price, currency, creditUnits)
  const pricedIn = price.creditUnit
  if (!pricedIn?.rate) return summary
  const unit = creditUnits?.find(u => u.id === pricedIn.creditUnitId)
  // The money a credit is billed at when the unit's wallet does not cover it, as on the plan page.
  return {
    ...summary,
    note: `1 ${unit?.name ?? 'credit'} = ${formatCurrencyNoRounding(pricedIn.rate, currency)}, billed when not covered by ${unit?.pluralName ?? 'credits'}`,
  }
}

function usagePriceSummary(
  price: Price,
  currency: string,
  creditUnits?: readonly CreditUnitNaming[]
): { model: string; amount: string } {
  if (price.pricing.case !== 'usagePricing') {
    return { model: '', amount: '' }
  }

  const usage = price.pricing.value
  const pricedIn = price.creditUnit
  const unit = pricedIn && creditUnits?.find(u => u.id === pricedIn.creditUnitId)
  const amountOf = (amount: string | number) => {
    if (!pricedIn) return formatCurrencyNoRounding(amount, currency)
    const n = Number(amount)
    const name = n === 1 ? (unit?.name ?? 'credit') : (unit?.pluralName ?? 'credits')
    return `${formatUsage(n)} ${name}`
  }

  switch (usage.model.case) {
    case 'perUnit':
      return {
        model: '',
        amount: `${amountOf(usage.model.value)}/unit`,
      }

    case 'tiered':
    case 'volume': {
      const rows = usage.model.value.rows || []
      if (rows.length === 0) {
        return { model: usage.model.case === 'tiered' ? 'Tiered' : 'Volume', amount: '-' }
      }
      const prices = rows.map(r => Number(r.unitPrice))
      const min = Math.min(...prices)
      const max = Math.max(...prices)
      const label = usage.model.case === 'tiered' ? 'Tiered' : 'Volume'
      const amount =
        min === max
          ? `${amountOf(min)}/unit`
          : `${amountOf(min)} – ${amountOf(max)}/unit`
      return { model: label, amount }
    }

    case 'package': {
      const pkg = usage.model.value
      return {
        model: 'Package',
        amount: `${amountOf(pkg.packagePrice)} / ${pkg.blockSize} units`,
      }
    }

    case 'matrix': {
      const rows = usage.model.value.rows || []
      if (rows.length === 0) {
        return { model: 'Matrix', amount: '-' }
      }
      const prices = rows.map(r => Number(r.perUnitPrice))
      const min = Math.min(...prices)
      const max = Math.max(...prices)
      const amount =
        min === max
          ? `${amountOf(min)}/unit`
          : `${amountOf(min)} – ${amountOf(max)}/unit`
      return { model: 'Matrix', amount }
    }

    default:
      return { model: 'Usage', amount: 'Variable' }
  }
}
