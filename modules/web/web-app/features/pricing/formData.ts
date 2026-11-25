import { FeeStructure_UsageModel, FeeType } from '@/rpc/api/prices/v1/models_pb'

import { pricesToFormData, toPricingTypeFromFeeType } from './conversions'
import { pricingDefaults } from './schemas'

import type { ComponentFeeType } from './conversions'
import type { CreditPackContext } from './schemas'
import type { CreditUnit } from '@/rpc/api/creditunits/v1/models_pb'
import type { FeeStructure, Price } from '@/rpc/api/prices/v1/models_pb'

// The form data a charge starts from: what its product is (its fee type and structure) and the
// fields its prices fill in. The charge forms (features/plans/charges) read and compile it.

/** What a product fixes about its charges: its metric, usage model, seat unit or billing. */
export interface StructuralInfo {
  metricId?: string
  usageModel?: string
  slotUnitName?: string
  billingType?: string
}

export function protoFeeTypeToComponentFeeType(feeType: FeeType): ComponentFeeType {
  switch (feeType) {
    case FeeType.RATE:
      return 'rate'
    case FeeType.SLOT:
      return 'slot'
    case FeeType.CAPACITY:
      return 'capacity'
    case FeeType.USAGE:
      return 'usage'
    case FeeType.EXTRA_RECURRING:
      return 'extraRecurring'
    case FeeType.ONE_TIME:
      return 'oneTime'
    case FeeType.CREDIT:
      return 'credit'
  }
}

export function extractStructuralInfo(
  feeType: ComponentFeeType | undefined,
  feeStructure: FeeStructure | undefined
): StructuralInfo {
  const structure = feeStructure?.structure
  if (!structure || !structure.case) return {}

  switch (feeType) {
    case 'slot':
      return structure.case === 'slot' ? { slotUnitName: structure.value.unitName } : {}
    case 'capacity':
      return structure.case === 'capacity' ? { metricId: structure.value.metricId } : {}
    case 'usage': {
      if (structure.case !== 'usage') return {}
      const usageModelMap: Record<number, string> = {
        [FeeStructure_UsageModel.PER_UNIT]: 'per_unit',
        [FeeStructure_UsageModel.TIERED]: 'tiered',
        [FeeStructure_UsageModel.VOLUME]: 'volume',
        [FeeStructure_UsageModel.PACKAGE]: 'package',
        [FeeStructure_UsageModel.MATRIX]: 'dimension_rates',
        [FeeStructure_UsageModel.DIMENSION_RATES]: 'dimension_rates',
        [FeeStructure_UsageModel.RATE_CARD]: 'rate_card',
      }
      return {
        metricId: structure.value.metricId,
        usageModel: usageModelMap[structure.value.model ?? 0],
      }
    }
    case 'extraRecurring':
      return structure.case === 'extraRecurring'
        ? { billingType: structure.value.billingType === 1 ? 'ADVANCE' : 'ARREAR' }
        : {}
    default:
      return {}
  }
}

export const creditPackContext = (
  currency: string,
  units: CreditUnit[] | undefined
): CreditPackContext => ({
  currency,
  unitPrecision: id => units?.find(unit => unit.id === id)?.precision,
})

function deriveUsageModelFromPrice(price?: Price): string | undefined {
  if (!price?.pricing || price.pricing.case !== 'usagePricing') return undefined
  return price.pricing.value.model.case
}

const usageModelProtoToForm: Record<string, string> = {
  perUnit: 'per_unit',
  tiered: 'tiered',
  volume: 'volume',
  package: 'package',
  dimensionRates: 'dimension_rates',
  matrix: 'dimension_rates',
  rateCard: 'rate_card',
}

export function buildDefaults(
  feeType: ComponentFeeType,
  existingPrice?: Price,
  isOverride?: boolean,
  usageModel?: string
): Record<string, unknown> {
  if (!existingPrice) {
    if (feeType === 'capacity' && isOverride) {
      return { term: 'MONTHLY', rate: '0.00', included: 0, overageRate: '0.00000000' }
    }
    switch (feeType) {
      case 'rate':
        return { term: 'MONTHLY', rate: '0.00' }
      case 'slot':
        return {
          slotUnitName: 'Seats',
          upgradePolicy: 'PRORATED',
          downgradePolicy: 'REMOVE_AT_END_OF_PERIOD',
          minimumCount: 1,
          term: 'MONTHLY',
          seatModel: 'per_unit',
          unitRate: '0.00',
        }
      case 'capacity':
        return {
          metricId: '',
          term: 'MONTHLY',
          thresholds: [{ included: 0, rate: '0.00', overageRate: '0.00000000' }],
        }
      case 'usage': {
        const model = usageModel ?? 'per_unit'
        return {
          metricId: '',
          usageModel: model,
          term: 'MONTHLY',
          ...pricingDefaults(toPricingTypeFromFeeType('usage', model)),
        }
      }
      case 'extraRecurring':
        return {
          term: 'MONTHLY',
          billingType: 'ADVANCE',
          unitPrice: '0.00',
          quantity: 1,
          quantityModel: 'per_unit',
        }
      case 'oneTime':
        return { unitPrice: '0.00', quantity: 1, quantityModel: 'per_unit' }
      case 'credit':
        return {
          term: 'MONTHLY',
          unitPrice: '0.00',
          quantity: 1,
          creditAmount: '0.00',
          grantCreditUnitId: '',
          expiryUnit: 'BILLING_PERIOD',
          expiryValue: 1,
          rolloverEnabled: false,
          rolloverMaxAmount: '',
          rolloverExpiryUnit: 'MONTH',
          rolloverExpiryValue: 1,
          once: false,
        }
    }
  }

  const priceUsageModel = deriveUsageModelFromPrice(existingPrice)
  const pricingType = toPricingTypeFromFeeType(feeType, priceUsageModel)
  const formData = pricesToFormData([existingPrice], pricingType)

  // For capacity overrides, flatten thresholds to a single threshold
  if (feeType === 'capacity' && isOverride && formData.thresholds) {
    const thresholds = formData.thresholds as { included: number; rate: string; overageRate: string }[]
    const t = thresholds[0] ?? { included: 0, rate: '0.00', overageRate: '0.00000000' }
    return {
      term: formData.term,
      rate: t.rate,
      included: t.included,
      overageRate: t.overageRate,
      creditUnitId: formData.creditUnitId ?? '',
      creditUnitRate: formData.creditUnitRate ?? '',
    }
  }

  if (feeType === 'usage' && priceUsageModel) {
    return { ...formData, usageModel: usageModelProtoToForm[priceUsageModel] ?? 'per_unit' }
  }

  return formData
}

export function buildDefaultsFromPrices(
  feeType: ComponentFeeType,
  prices: Price[]
): Record<string, unknown> {
  if (prices.length === 0) return buildDefaults(feeType)

  const usageModel = deriveUsageModelFromPrice(prices[0])
  const pricingType = toPricingTypeFromFeeType(feeType, usageModel)
  const formData = pricesToFormData(prices, pricingType)

  if (feeType === 'usage' && usageModel) {
    return { ...formData, usageModel: usageModelProtoToForm[usageModel] ?? 'per_unit' }
  }

  return formData
}
