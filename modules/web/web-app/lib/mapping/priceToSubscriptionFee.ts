import { Price } from '@/rpc/api/prices/v1/models_pb'
import { UsageFee } from '@/rpc/api/pricecomponents/v1/models_pb'
import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'
import { formatCurrencyNoRounding } from '@/lib/utils/numbers'
import {
  SubscriptionFee,
  SubscriptionFee_CapacitySubscriptionFee,
  SubscriptionFee_ExtraRecurringSubscriptionFee,
  SubscriptionFee_OneTimeSubscriptionFee,
  SubscriptionFee_RateSubscriptionFee,
  SubscriptionFee_SlotSubscriptionFee,
  SubscriptionFeeBillingPeriod,
} from '@/rpc/api/subscriptions/v1/models_pb'
import { FeeStructure_BillingType } from '@/rpc/api/prices/v1/models_pb'

import { PriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb'

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

/**
 * Convert a Price to a SubscriptionFee.
 *
 * For usage and capacity, metricId is left empty — the backend resolves
 * it from the price component's linked product during subscription creation.
 */
export function priceToSubscriptionFee(
  price: Price,
  config?: { initialSlotCount?: number }
): SubscriptionFee {
  const fee = new SubscriptionFee()

  switch (price.pricing.case) {
    case 'ratePricing':
      fee.fee = {
        case: 'rate',
        value: new SubscriptionFee_RateSubscriptionFee({
          rate: price.pricing.value.rate,
        }),
      }
      break

    case 'slotPricing':
      fee.fee = {
        case: 'slot',
        value: new SubscriptionFee_SlotSubscriptionFee({
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
        value: new SubscriptionFee_CapacitySubscriptionFee({
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
      const usageFee = new UsageFee({ term: price.cadence, metricId: '' })
      usageFee.model = usage.model
      fee.fee = { case: 'usage', value: usageFee }
      break
    }

    case 'oneTimePricing': {
      const ot = price.pricing.value
      const total = (parseFloat(ot.unitPrice || '0') * (ot.quantity || 1)).toString()
      fee.fee = {
        case: 'oneTime',
        value: new SubscriptionFee_OneTimeSubscriptionFee({
          rate: ot.unitPrice,
          quantity: ot.quantity || 1,
          total,
        }),
      }
      break
    }

    case 'extraRecurringPricing': {
      const er = price.pricing.value
      const total = (parseFloat(er.unitPrice || '0') * (er.quantity || 1)).toString()
      fee.fee = {
        case: 'recurring',
        value: new SubscriptionFee_ExtraRecurringSubscriptionFee({
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
  }

  return {
    unitPrice,
    quantity,
    total: unitPrice * quantity,
    isMetered: false,
  }
}

/**
 * Get billing period label from a Price's cadence.
 */
export function getBillingPeriodLabel(period: BillingPeriod): string {
  switch (period) {
    case BillingPeriod.MONTHLY:
      return 'Monthly'
    case BillingPeriod.QUARTERLY:
      return 'Quarterly'
    case BillingPeriod.SEMIANNUAL:
      return 'Semiannual'
    case BillingPeriod.ANNUAL:
      return 'Annual'
    default:
      return 'Monthly'
  }
}

export function getPriceBillingLabel(price: Price): string {
  if (price.pricing.case === 'oneTimePricing') {
    return 'One-time'
  }
  return getBillingPeriodLabel(price.cadence)
}

/**
 * Format a usage-based Price for display, showing the pricing model and rate summary.
 */
export function formatUsagePriceSummary(
  price: Price,
  currency: string
): { model: string; amount: string } {
  if (price.pricing.case !== 'usagePricing') {
    return { model: '', amount: '' }
  }

  const usage = price.pricing.value

  switch (usage.model.case) {
    case 'perUnit':
      return {
        model: '',
        amount: `${formatCurrencyNoRounding(usage.model.value, currency)}/unit`,
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
          ? `${formatCurrencyNoRounding(min, currency)}/unit`
          : `${formatCurrencyNoRounding(min, currency)} – ${formatCurrencyNoRounding(max, currency)}/unit`
      return { model: label, amount }
    }

    case 'package': {
      const pkg = usage.model.value
      return {
        model: 'Package',
        amount: `${formatCurrencyNoRounding(pkg.packagePrice, currency)} / ${pkg.blockSize} units`,
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
          ? `${formatCurrencyNoRounding(min, currency)}/unit`
          : `${formatCurrencyNoRounding(min, currency)} – ${formatCurrencyNoRounding(max, currency)}/unit`
      return { model: 'Matrix', amount }
    }

    default:
      return { model: 'Usage', amount: 'Variable' }
  }
}
