import { FeeType, Price } from '@/rpc/api/prices/v1/models_pb'
import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'

export function feeTypeLabel(feeType: FeeType): string {
  switch (feeType) {
    case FeeType.RATE:
      return 'Rate'
    case FeeType.SLOT:
      return 'Slot'
    case FeeType.CAPACITY:
      return 'Capacity'
    case FeeType.USAGE:
      return 'Usage'
    case FeeType.EXTRA_RECURRING:
      return 'Recurring'
    case FeeType.ONE_TIME:
      return 'One-time'
    default:
      return 'Unknown'
  }
}

export function formatCadence(cadence: BillingPeriod): string {
  switch (cadence) {
    case BillingPeriod.MONTHLY:
      return 'Monthly'
    case BillingPeriod.QUARTERLY:
      return 'Quarterly'
    case BillingPeriod.SEMIANNUAL:
      return 'Semiannual'
    case BillingPeriod.ANNUAL:
      return 'Annual'
    default:
      return 'Unknown'
  }
}

export function formatPricingSummary(price: Price): string {
  switch (price.pricing.case) {
    case 'ratePricing':
      return `${price.pricing.value.rate}/${formatCadence(price.cadence).toLowerCase()}`
    case 'slotPricing':
      return `${price.pricing.value.unitRate}/unit/${formatCadence(price.cadence).toLowerCase()}`
    case 'capacityPricing':
      return `${price.pricing.value.rate} (${price.pricing.value.included} included)`
    case 'usagePricing': {
      const model = price.pricing.value.model
      switch (model.case) {
        case 'perUnit':
          return `${model.value}/unit`
        case 'tiered':
          return `Tiered (${model.value.rows.length} tiers)`
        case 'volume':
          return `Volume (${model.value.rows.length} tiers)`
        case 'package':
          return `${model.value.packagePrice}/pkg`
        case 'matrix':
          return `Matrix (${model.value.rows.length} rows)`
        default:
          return 'Usage'
      }
    }
    case 'extraRecurringPricing':
      return `${price.pricing.value.unitPrice} × ${price.pricing.value.quantity}`
    case 'oneTimePricing':
      return `${price.pricing.value.unitPrice} × ${price.pricing.value.quantity}`
    default:
      return '-'
  }
}
