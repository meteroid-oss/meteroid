import { create } from '@bufbuild/protobuf';

import {
  CapacityPricingSchema,
  ExtraRecurringPricingSchema,
  OneTimePricingSchema,
  RatePricingSchema,
  SlotPricingSchema,
  UsagePricingSchema,
  UsagePricing_MatrixPricingSchema,
  UsagePricing_MatrixPricing_MatrixDimensionSchema,
  UsagePricing_MatrixPricing_MatrixRowSchema,
  UsagePricing_PackagePricingSchema,
  UsagePricing_TieredAndVolumePricingSchema,
  UsagePricing_TieredAndVolumePricing_TierRowSchema,
  type Price,
} from '@/rpc/api/prices/v1/models_pb';


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
        value: create(RatePricingSchema, { rate: d.rate }),
      };
    }
    case 'slot': {
      const d = data as SlotPricingData
      return {
        case: 'slotPricing',
        value: create(
          SlotPricingSchema,
          { unitRate: d.unitRate, minSlots: d.minSlots, maxSlots: d.maxSlots }
        ),
      };
    }
    case 'capacity': {
      const d = data as CapacityPricingData
      return {
        case: 'capacityPricing',
        value: create(CapacityPricingSchema, {
          rate: d.rate,
          included: BigInt(d.included),
          overageRate: d.overageRate,
        }),
      };
    }
    case 'perUnit': {
      const d = data as PerUnitPricingData
      return {
        case: 'usagePricing',
        value: create(UsagePricingSchema, {
          model: { case: 'perUnit', value: d.unitPrice },
        }),
      };
    }
    case 'tiered': {
      const d = data as TieredPricingData
      return {
        case: 'usagePricing',
        value: create(UsagePricingSchema, {
          model: {
            case: 'tiered',
            value: create(UsagePricing_TieredAndVolumePricingSchema, {
              rows: d.rows.map(
                r =>
                  create(UsagePricing_TieredAndVolumePricing_TierRowSchema, {
                    firstUnit: BigInt(r.firstUnit),
                    unitPrice: r.unitPrice,
                    flatFee: r.flatFee,
                    flatCap: r.flatCap,
                  })
              ),
            }),
          },
        }),
      };
    }
    case 'volume': {
      const d = data as TieredPricingData
      return {
        case: 'usagePricing',
        value: create(UsagePricingSchema, {
          model: {
            case: 'volume',
            value: create(UsagePricing_TieredAndVolumePricingSchema, {
              rows: d.rows.map(
                r =>
                  create(UsagePricing_TieredAndVolumePricing_TierRowSchema, {
                    firstUnit: BigInt(r.firstUnit),
                    unitPrice: r.unitPrice,
                    flatFee: r.flatFee,
                    flatCap: r.flatCap,
                  })
              ),
            }),
          },
        }),
      };
    }
    case 'package': {
      const d = data as PackagePricingData
      return {
        case: 'usagePricing',
        value: create(UsagePricingSchema, {
          model: {
            case: 'package',
            value: create(UsagePricing_PackagePricingSchema, {
              packagePrice: d.packagePrice,
              blockSize: BigInt(d.blockSize),
            }),
          },
        }),
      };
    }
    case 'matrix': {
      const d = data as MatrixPricingData
      return {
        case: 'usagePricing',
        value: create(UsagePricingSchema, {
          model: {
            case: 'matrix',
            value: create(UsagePricing_MatrixPricingSchema, {
              rows: d.rows.map(
                r =>
                  create(UsagePricing_MatrixPricing_MatrixRowSchema, {
                    perUnitPrice: r.perUnitPrice,
                    dimension1: create(UsagePricing_MatrixPricing_MatrixDimensionSchema, r.dimension1),
                    dimension2: r.dimension2
                      ? create(UsagePricing_MatrixPricing_MatrixDimensionSchema, r.dimension2)
                      : undefined,
                  })
              ),
            }),
          },
        }),
      };
    }
    case 'extraRecurring': {
      const d = data as ExtraRecurringPricingData
      return {
        case: 'extraRecurringPricing',
        value: create(ExtraRecurringPricingSchema, {
          unitPrice: d.unitPrice,
          quantity: d.quantity,
        }),
      };
    }
    case 'oneTime': {
      const d = data as OneTimePricingData
      return {
        case: 'oneTimePricing',
        value: create(OneTimePricingSchema, {
          unitPrice: d.unitPrice,
          quantity: d.quantity,
        }),
      };
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
