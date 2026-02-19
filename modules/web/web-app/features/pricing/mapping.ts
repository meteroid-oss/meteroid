import {
  CapacityPricing,
  ExtraRecurringPricing,
  OneTimePricing,
  RatePricing,
  SlotPricing,
  UsagePricing,
  UsagePricing_MatrixPricing,
  UsagePricing_MatrixPricing_MatrixDimension,
  UsagePricing_MatrixPricing_MatrixRow,
  UsagePricing_PackagePricing,
  UsagePricing_TieredAndVolumePricing,
  UsagePricing_TieredAndVolumePricing_TierRow,
  type Price,
} from '@/rpc/api/prices/v1/models_pb'

import type {
  CapacityPricingData,
  ExtraRecurringPricingData,
  MatrixPricingData,
  OneTimePricingData,
  PackagePricingData,
  PerUnitPricingData,
  PricingType,
  RatePricingData,
  SlotPricingData,
  TieredPricingData,
} from './schemas'

/**
 * Convert form pricing data to proto pricing oneof.
 * Returns an object suitable for setting on CreatePriceRequest.pricing or PriceInput.pricing.
 */
export function formDataToProtoPricing(
  pricingType: PricingType,
  data: Record<string, unknown>
): Price['pricing'] {
  switch (pricingType) {
    case 'rate': {
      const d = data as RatePricingData
      return {
        case: 'ratePricing',
        value: new RatePricing({ rate: d.rate }),
      }
    }
    case 'slot': {
      const d = data as SlotPricingData
      return {
        case: 'slotPricing',
        value: new SlotPricing({ unitRate: d.unitRate, minSlots: d.minSlots, maxSlots: d.maxSlots }),
      }
    }
    case 'capacity': {
      const d = data as CapacityPricingData
      return {
        case: 'capacityPricing',
        value: new CapacityPricing({
          rate: d.rate,
          included: BigInt(d.included),
          overageRate: d.overageRate,
        }),
      }
    }
    case 'perUnit': {
      const d = data as PerUnitPricingData
      return {
        case: 'usagePricing',
        value: new UsagePricing({
          model: { case: 'perUnit', value: d.unitPrice },
        }),
      }
    }
    case 'tiered': {
      const d = data as TieredPricingData
      return {
        case: 'usagePricing',
        value: new UsagePricing({
          model: {
            case: 'tiered',
            value: new UsagePricing_TieredAndVolumePricing({
              rows: d.rows.map(
                r =>
                  new UsagePricing_TieredAndVolumePricing_TierRow({
                    firstUnit: BigInt(r.firstUnit),
                    unitPrice: r.unitPrice,
                    flatFee: r.flatFee,
                    flatCap: r.flatCap,
                  })
              ),
            }),
          },
        }),
      }
    }
    case 'volume': {
      const d = data as TieredPricingData
      return {
        case: 'usagePricing',
        value: new UsagePricing({
          model: {
            case: 'volume',
            value: new UsagePricing_TieredAndVolumePricing({
              rows: d.rows.map(
                r =>
                  new UsagePricing_TieredAndVolumePricing_TierRow({
                    firstUnit: BigInt(r.firstUnit),
                    unitPrice: r.unitPrice,
                    flatFee: r.flatFee,
                    flatCap: r.flatCap,
                  })
              ),
            }),
          },
        }),
      }
    }
    case 'package': {
      const d = data as PackagePricingData
      return {
        case: 'usagePricing',
        value: new UsagePricing({
          model: {
            case: 'package',
            value: new UsagePricing_PackagePricing({
              packagePrice: d.packagePrice,
              blockSize: BigInt(d.blockSize),
            }),
          },
        }),
      }
    }
    case 'matrix': {
      const d = data as MatrixPricingData
      return {
        case: 'usagePricing',
        value: new UsagePricing({
          model: {
            case: 'matrix',
            value: new UsagePricing_MatrixPricing({
              rows: d.rows.map(
                r =>
                  new UsagePricing_MatrixPricing_MatrixRow({
                    perUnitPrice: r.perUnitPrice,
                    dimension1: new UsagePricing_MatrixPricing_MatrixDimension(r.dimension1),
                    dimension2: r.dimension2
                      ? new UsagePricing_MatrixPricing_MatrixDimension(r.dimension2)
                      : undefined,
                  })
              ),
            }),
          },
        }),
      }
    }
    case 'extraRecurring': {
      const d = data as ExtraRecurringPricingData
      return {
        case: 'extraRecurringPricing',
        value: new ExtraRecurringPricing({
          unitPrice: d.unitPrice,
          quantity: d.quantity,
        }),
      }
    }
    case 'oneTime': {
      const d = data as OneTimePricingData
      return {
        case: 'oneTimePricing',
        value: new OneTimePricing({
          unitPrice: d.unitPrice,
          quantity: d.quantity,
        }),
      }
    }
  }
}

/**
 * Convert proto pricing oneof to form data for editing.
 */
export function protoPricingToFormData(pricing: Price['pricing']): {
  pricingType: PricingType
  data: Record<string, unknown>
} | null {
  switch (pricing.case) {
    case 'ratePricing':
      return {
        pricingType: 'rate',
        data: { rate: pricing.value.rate },
      }
    case 'slotPricing':
      return {
        pricingType: 'slot',
        data: { unitRate: pricing.value.unitRate, minSlots: pricing.value.minSlots, maxSlots: pricing.value.maxSlots },
      }
    case 'capacityPricing':
      return {
        pricingType: 'capacity',
        data: {
          rate: pricing.value.rate,
          included: Number(pricing.value.included),
          overageRate: pricing.value.overageRate,
        },
      }
    case 'usagePricing': {
      const model = pricing.value.model
      switch (model.case) {
        case 'perUnit':
          return {
            pricingType: 'perUnit',
            data: { unitPrice: model.value },
          }
        case 'tiered':
          return {
            pricingType: 'tiered',
            data: {
              rows: model.value.rows.map(r => ({
                firstUnit: r.firstUnit,
                unitPrice: r.unitPrice,
                flatFee: r.flatFee,
                flatCap: r.flatCap,
              })),
            },
          }
        case 'volume':
          return {
            pricingType: 'volume',
            data: {
              rows: model.value.rows.map(r => ({
                firstUnit: r.firstUnit,
                unitPrice: r.unitPrice,
                flatFee: r.flatFee,
                flatCap: r.flatCap,
              })),
            },
          }
        case 'package':
          return {
            pricingType: 'package',
            data: {
              packagePrice: model.value.packagePrice,
              blockSize: Number(model.value.blockSize),
            },
          }
        case 'matrix':
          return {
            pricingType: 'matrix',
            data: {
              rows: model.value.rows.map(r => ({
                perUnitPrice: r.perUnitPrice,
                dimension1: r.dimension1
                  ? { key: r.dimension1.key, value: r.dimension1.value }
                  : { key: '', value: '' },
                dimension2: r.dimension2
                  ? { key: r.dimension2.key, value: r.dimension2.value }
                  : undefined,
              })),
            },
          }
        default:
          return null
      }
    }
    case 'extraRecurringPricing':
      return {
        pricingType: 'extraRecurring',
        data: {
          unitPrice: pricing.value.unitPrice,
          quantity: pricing.value.quantity,
        },
      }
    case 'oneTimePricing':
      return {
        pricingType: 'oneTime',
        data: {
          unitPrice: pricing.value.unitPrice,
          quantity: pricing.value.quantity,
        },
      }
    default:
      return null
  }
}
