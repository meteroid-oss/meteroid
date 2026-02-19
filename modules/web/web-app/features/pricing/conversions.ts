import {
  PriceInput,
  NewProduct,
  ProductRef,
  PriceEntry,
} from '@/rpc/api/pricecomponents/v1/models_pb'
import {
  FeeType as ProtoFeeType,
  FeeStructure,
  FeeStructure_BillingType,
  FeeStructure_CapacityStructure,
  FeeStructure_ExtraRecurringStructure,
  FeeStructure_OneTimeStructure,
  FeeStructure_RateStructure,
  FeeStructure_SlotStructure,
  FeeStructure_UsageModel,
  FeeStructure_UsageStructure,
  Price,
} from '@/rpc/api/prices/v1/models_pb'
import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'

import { formDataToProtoPricing, protoPricingToFormData } from './mapping'
import { pricingDefaults } from './schemas'

import type { PricingType } from './schemas'

// --- Cadence helpers ---

type Cadence = 'MONTHLY' | 'QUARTERLY' | 'SEMIANNUAL' | 'ANNUAL'

function cadenceToProto(cadence: Cadence): BillingPeriod {
  switch (cadence) {
    case 'MONTHLY':
      return BillingPeriod.MONTHLY
    case 'QUARTERLY':
      return BillingPeriod.QUARTERLY
    case 'SEMIANNUAL':
      return BillingPeriod.SEMIANNUAL
    case 'ANNUAL':
      return BillingPeriod.ANNUAL
  }
}

function protoToCadence(period: BillingPeriod): Cadence {
  switch (period) {
    case BillingPeriod.MONTHLY:
      return 'MONTHLY'
    case BillingPeriod.QUARTERLY:
      return 'QUARTERLY'
    case BillingPeriod.SEMIANNUAL:
      return 'SEMIANNUAL'
    case BillingPeriod.ANNUAL:
      return 'ANNUAL'
  }
}

// --- Fee type helpers ---

export type ComponentFeeType = 'rate' | 'slot' | 'capacity' | 'usage' | 'extraRecurring' | 'oneTime'

export function feeTypeToProto(feeType: ComponentFeeType): ProtoFeeType {
  switch (feeType) {
    case 'rate':
      return ProtoFeeType.RATE
    case 'slot':
      return ProtoFeeType.SLOT
    case 'capacity':
      return ProtoFeeType.CAPACITY
    case 'usage':
      return ProtoFeeType.USAGE
    case 'extraRecurring':
      return ProtoFeeType.EXTRA_RECURRING
    case 'oneTime':
      return ProtoFeeType.ONE_TIME
  }
}

/**
 * Derive PricingType from ComponentFeeType + optional usage model.
 */
export function toPricingTypeFromFeeType(
  feeType: ComponentFeeType,
  usageModel?: string
): PricingType {
  if (feeType === 'usage') {
    switch (usageModel) {
      case 'per_unit':
        return 'perUnit'
      case 'tiered':
        return 'tiered'
      case 'volume':
        return 'volume'
      case 'package':
        return 'package'
      case 'matrix':
        return 'matrix'
      default:
        return 'perUnit'
    }
  }
  switch (feeType) {
    case 'rate':
      return 'rate'
    case 'slot':
      return 'slot'
    case 'capacity':
      return 'capacity'
    case 'extraRecurring':
      return 'extraRecurring'
    case 'oneTime':
      return 'oneTime'
  }
}

/**
 * Build PriceInput[] proto messages from form data.
 *
 * For capacity: one PriceInput per threshold (all share the same cadence)
 * For other cadenced types: single PriceInput with the term cadence
 * For one-time: no cadence concept, use MONTHLY as default
 */
export function buildPriceInputs(
  pricingType: PricingType,
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  formData: Record<string, any>,
  currency: string
): PriceInput[] {
  // Capacity: one PriceInput per threshold row
  if (pricingType === 'capacity') {
    const cadence = formData.term as Cadence
    const thresholds = formData.thresholds as { included: number; rate: string; overageRate: string }[]
    return thresholds.map(t => {
      const protoPricing = formDataToProtoPricing('capacity', {
        rate: t.rate,
        included: t.included,
        overageRate: t.overageRate,
      })
      return new PriceInput({
        cadence: cadenceToProto(cadence),
        currency,
        pricing: protoPricing,
      })
    })
  }

  // Single-cadence types (rate, slot, usage variants, extraRecurring)
  if (
    pricingType === 'rate' ||
    pricingType === 'slot' ||
    pricingType === 'perUnit' ||
    pricingType === 'tiered' ||
    pricingType === 'volume' ||
    pricingType === 'package' ||
    pricingType === 'matrix' ||
    pricingType === 'extraRecurring'
  ) {
    const cadence = formData.term as Cadence
    const protoPricing = formDataToProtoPricing(pricingType, formData)
    return [
      new PriceInput({
        cadence: cadenceToProto(cadence),
        currency,
        pricing: protoPricing,
      }),
    ]
  }

  // One-time: no cadence concept, use MONTHLY as default
  const protoPricing = formDataToProtoPricing(pricingType, formData)
  return [
    new PriceInput({
      cadence: BillingPeriod.MONTHLY,
      currency,
      pricing: protoPricing,
    }),
  ]
}

/**
 * Build a FeeStructure proto for new product creation.
 */
export function buildFeeStructure(
  feeType: ComponentFeeType,
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  formData: Record<string, any>
): FeeStructure {
  switch (feeType) {
    case 'rate':
      return new FeeStructure({
        structure: { case: 'rate', value: new FeeStructure_RateStructure() },
      })
    case 'slot':
      return new FeeStructure({
        structure: {
          case: 'slot',
          value: new FeeStructure_SlotStructure({
            unitName: formData.slotUnitName ?? 'Seats',
          }),
        },
      })
    case 'capacity':
      return new FeeStructure({
        structure: {
          case: 'capacity',
          value: new FeeStructure_CapacityStructure({
            metricId: formData.metricId,
          }),
        },
      })
    case 'usage': {
      const usageModelMap: Record<string, FeeStructure_UsageModel> = {
        per_unit: FeeStructure_UsageModel.PER_UNIT,
        tiered: FeeStructure_UsageModel.TIERED,
        volume: FeeStructure_UsageModel.VOLUME,
        package: FeeStructure_UsageModel.PACKAGE,
        matrix: FeeStructure_UsageModel.MATRIX,
      }
      return new FeeStructure({
        structure: {
          case: 'usage',
          value: new FeeStructure_UsageStructure({
            metricId: formData.metricId,
            model: usageModelMap[formData.usageModel ?? 'per_unit'] ?? FeeStructure_UsageModel.PER_UNIT,
          }),
        },
      })
    }
    case 'extraRecurring': {
      const billingType =
        formData.billingType === 'ADVANCE'
          ? FeeStructure_BillingType.ADVANCE
          : FeeStructure_BillingType.ARREAR
      return new FeeStructure({
        structure: {
          case: 'extraRecurring',
          value: new FeeStructure_ExtraRecurringStructure({ billingType }),
        },
      })
    }
    case 'oneTime':
      return new FeeStructure({
        structure: { case: 'oneTime', value: new FeeStructure_OneTimeStructure() },
      })
  }
}

/**
 * Convert Price[] proto objects to form data for editing.
 *
 * For capacity: collects all prices into thresholds array
 * For other cadenced types: uses first price's cadence + pricing data
 * For one-time: returns pricing data only
 */
export function pricesToFormData(
  prices: Price[],
  pricingType: PricingType
): Record<string, unknown> {
  if (prices.length === 0) {
    return pricingDefaults(pricingType)
  }

  // Capacity: collect all prices into thresholds
  if (pricingType === 'capacity') {
    const thresholds = prices
      .filter(p => p.pricing.case === 'capacityPricing')
      .map(p => {
        const v = p.pricing.value as { rate: string; included: bigint; overageRate: string }
        return {
          included: Number(v.included),
          rate: v.rate,
          overageRate: v.overageRate,
        }
      })
    return {
      term: protoToCadence(prices[0].cadence),
      thresholds: thresholds.length > 0 ? thresholds : [{ included: 0, rate: '0.00', overageRate: '0.00000000' }],
    }
  }

  // Single-cadence and one-time: take the first price
  const price = prices[0]
  const parsed = protoPricingToFormData(price.pricing)
  if (!parsed) return pricingDefaults(pricingType)

  if (pricingType === 'oneTime') {
    return parsed.data
  }

  return {
    term: protoToCadence(price.cadence),
    ...parsed.data,
  }
}

/**
 * Build a ProductRef for new product creation (inline with component).
 */
export function buildNewProductRef(
  name: string,
  feeType: ComponentFeeType,
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  formData: Record<string, any>
): ProductRef {
  return new ProductRef({
    ref: {
      case: 'newProduct',
      value: new NewProduct({
        name,
        feeType: feeTypeToProto(feeType),
        feeStructure: buildFeeStructure(feeType, formData),
      }),
    },
  })
}

/**
 * Build a ProductRef referencing an existing product by ID.
 */
export function buildExistingProductRef(productId: string): ProductRef {
  return new ProductRef({
    ref: { case: 'existingProductId', value: productId },
  })
}

/**
 * Wrap PriceInput[] as PriceEntry[] (all new prices).
 */
export function wrapAsNewPriceEntries(inputs: PriceInput[]): PriceEntry[] {
  return inputs.map(
    pi => new PriceEntry({ entry: { case: 'newPrice', value: pi } })
  )
}

/**
 * Wrap existing price IDs as PriceEntry[].
 */
export function existingPriceEntries(priceIds: string[]): PriceEntry[] {
  return priceIds.map(
    id => new PriceEntry({ entry: { case: 'existingPriceId', value: id } })
  )
}

/**
 * Build a local Price proto from form data.
 * Used when creating extra subscription components or overrides (no server round-trip).
 */
export function formDataToPrice(
  feeType: ComponentFeeType,
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  formData: Record<string, any>,
  currency: string
): Price {
  const pricingType = toPricingTypeFromFeeType(
    feeType,
    feeType === 'usage' ? formData.usageModel : undefined
  )
  const cadence = formData.term
    ? cadenceToProto(formData.term as Cadence)
    : BillingPeriod.MONTHLY
  const pricing = formDataToProtoPricing(pricingType, formData)
  return new Price({ cadence, currency, pricing })
}

/**
 * Derive the ComponentFeeType from a Price's pricing oneof.
 */
export function feeTypeFromPrice(price: Price): ComponentFeeType {
  switch (price.pricing.case) {
    case 'ratePricing':
      return 'rate'
    case 'slotPricing':
      return 'slot'
    case 'capacityPricing':
      return 'capacity'
    case 'usagePricing':
      return 'usage'
    case 'extraRecurringPricing':
      return 'extraRecurring'
    case 'oneTimePricing':
      return 'oneTime'
    default:
      return 'rate'
  }
}
