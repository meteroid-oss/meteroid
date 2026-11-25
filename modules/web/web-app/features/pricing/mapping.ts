import { create } from '@bufbuild/protobuf';

import { markupToMultiplier, multiplierToMarkup } from '@/features/productCatalog/ratecards/markup';
import {
  CapacityPricingSchema,
  CreditPackExpiry_Unit,
  CreditPricingSchema,
  ExtraRecurringPricingSchema,
  OneTimePricingSchema,
  RatePricingSchema,
  SlotPricingSchema,
  UsagePricingSchema,
  UsagePricing_DimensionRatesPricingSchema,
  UsagePricing_DimensionRatesPricing_DimensionRateSchema,
  UsagePricing_RateCardPricingSchema,
  UsagePricing_PackagePricingSchema,
  UsagePricing_TieredAndVolumePricingSchema,
  UsagePricing_TieredAndVolumePricing_TierRowSchema,
  type Price,
  type UsagePricing,
  type UsagePricing_DimensionRatesPricing,
  type UsagePricing_MatrixPricing,
} from '@/rpc/api/prices/v1/models_pb';
import { RateCardUnmatched } from '@/rpc/api/ratecards/v1/models_pb';

import {
  packRollsOver,
  type CapacityPricingData,
  type CreditPricingData,
  type DimensionRatesPricingData,
  type ExtraRecurringPricingData,
  type RateCardPricingData,
  type OneTimePricingData,
  type PackagePricingData,
  type PerUnitPricingData,
  type PricingType,
  type QuantityModel,
  type RatePricingData,
  type SlotPricingData,
  type TieredPricingData,
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
      const seatModel = d.seatModel ?? 'per_unit'
      const priced =
        seatModel === 'per_unit'
          ? undefined
          : formDataToProtoPricing(seatModel, data as Record<string, unknown>)
      return {
        case: 'slotPricing',
        value: create(SlotPricingSchema, {
          unitRate: seatModel === 'per_unit' ? d.unitRate : '',
          minSlots: d.minSlots,
          maxSlots: d.maxSlots,
          model: priced?.case === 'usagePricing' ? priced.value : undefined,
        }),
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
    case 'dimensionRates': {
      const d = data as DimensionRatesPricingData
      const dimensions = d.dimensions.map(dimension => dimension.trim())
      return {
        case: 'usagePricing',
        value: create(UsagePricingSchema, {
          model: {
            case: 'dimensionRates',
            value: create(UsagePricing_DimensionRatesPricingSchema, {
              dimensions,
              // A cell left empty matches any value.
              rates: d.keyRates.map(r =>
                create(UsagePricing_DimensionRatesPricing_DimensionRateSchema, {
                  key: dimensions.map((_, i) => r.key[i]?.trim() || '*'),
                  rate: r.rate,
                  label: r.label?.trim() || undefined,
                })
              ),
              unitSize: d.unitSize?.trim() || '1',
              onUnmatched:
                d.onUnmatched === 'ignore' ? RateCardUnmatched.IGNORE : RateCardUnmatched.HOLD,
            }),
          },
        }),
      };
    }
    case 'rateCard': {
      const d = data as RateCardPricingData
      return {
        case: 'usagePricing',
        value: create(UsagePricingSchema, {
          model: {
            case: 'rateCard',
            value: create(UsagePricing_RateCardPricingSchema, {
              rateCardId: d.rateCardId,
              multiplier: markupToMultiplier(d.markup),
            }),
          },
        }),
      };
    }
    case 'extraRecurring': {
      const d = data as ExtraRecurringPricingData
      const model = quantityModelToProto(data)
      return {
        case: 'extraRecurringPricing',
        value: create(ExtraRecurringPricingSchema, {
          unitPrice: model ? '0' : d.unitPrice,
          quantity: d.quantity,
          model,
        }),
      };
    }
    case 'oneTime': {
      const d = data as OneTimePricingData
      const model = quantityModelToProto(data)
      return {
        case: 'oneTimePricing',
        value: create(OneTimePricingSchema, {
          unitPrice: model ? '0' : d.unitPrice,
          quantity: d.quantity,
          model,
        }),
      };
    }
    case 'credit': {
      const d = data as CreditPricingData
      return {
        case: 'creditPricing',
        value: create(CreditPricingSchema, {
          unitPrice: d.unitPrice,
          quantity: d.quantity,
          creditAmount: d.creditAmount,
          creditUnitId: d.grantCreditUnitId || undefined,
          expiry:
            d.expiryUnit === 'NONE'
              ? undefined
              : {
                  unit: CreditPackExpiry_Unit[d.expiryUnit],
                  value: d.expiryUnit === 'BILLING_PERIOD' ? 1 : d.expiryValue,
                },
          // Only an included (free) pack that expires rolls over; the switch is disabled otherwise.
          rollover:
            packRollsOver(d)
              ? {
                  maxAmount: d.rolloverMaxAmount.trim() === '' ? undefined : d.rolloverMaxAmount,
                  expiry: {
                    unit: CreditPackExpiry_Unit[d.rolloverExpiryUnit],
                    value: d.rolloverExpiryValue,
                  },
                }
              : undefined,
          once: d.once,
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
    case 'slotPricing': {
      const seats = { minSlots: pricing.value.minSlots, maxSlots: pricing.value.maxSlots }
      const model = pricing.value.model
      const priced = model
        ? protoPricingToFormData({ case: 'usagePricing', value: model })
        : null
      if (!priced) {
        return {
          pricingType: 'slot',
          data: { seatModel: 'per_unit', unitRate: pricing.value.unitRate, ...seats },
        }
      }
      return {
        pricingType: 'slot',
        data: { seatModel: priced.pricingType, ...priced.data, ...seats },
      }
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
        case 'rateCard':
          return {
            pricingType: 'rateCard',
            data: {
              rateCardId: model.value.rateCardId,
              markup: multiplierToMarkup(model.value.multiplier),
            },
          }
        case 'dimensionRates':
          return { pricingType: 'dimensionRates', data: dimensionRatesToForm(model.value) }
        // A matrix is read by the server as rates by dimension; kept for data cached before.
        case 'matrix':
          return { pricingType: 'dimensionRates', data: dimensionRatesToForm(matrixAsRates(model.value)) }
        default:
          return null
      }
    }
    case 'extraRecurringPricing':
      return {
        pricingType: 'extraRecurring',
        data: {
          quantity: pricing.value.quantity,
          ...quantityModelToForm(pricing.value.model, pricing.value.unitPrice),
        },
      }
    case 'oneTimePricing':
      return {
        pricingType: 'oneTime',
        data: {
          quantity: pricing.value.quantity,
          ...quantityModelToForm(pricing.value.model, pricing.value.unitPrice),
        },
      }
    case 'creditPricing': {
      const expiry = pricing.value.expiry
      const rollover = pricing.value.rollover
      const rolloverUnit = rollover?.expiry ? CreditPackExpiry_Unit[rollover.expiry.unit] : undefined
      return {
        pricingType: 'credit',
        data: {
          unitPrice: pricing.value.unitPrice,
          quantity: pricing.value.quantity,
          creditAmount: pricing.value.creditAmount,
          grantCreditUnitId: pricing.value.creditUnitId ?? '',
          expiryUnit: expiry ? CreditPackExpiry_Unit[expiry.unit] : 'NONE',
          expiryValue: expiry?.value ?? 1,
          rolloverEnabled: rollover !== undefined,
          rolloverMaxAmount: rollover?.maxAmount ?? '',
          rolloverExpiryUnit: rolloverUnit === 'DAY' ? 'DAY' : 'MONTH',
          rolloverExpiryValue: rollover?.expiry?.value ?? 1,
          once: pricing.value.once,
        },
      }
    }
    default:
      return null
  }
}

/** The model of a fixed count (extra recurring, one-time); none means unit price × quantity. */
function quantityModelToProto(data: Record<string, unknown>): UsagePricing | undefined {
  const model = (data.quantityModel as QuantityModel | undefined) ?? 'per_unit'
  if (model === 'per_unit') return undefined
  const priced = formDataToProtoPricing(model, data)
  return priced.case === 'usagePricing' ? priced.value : undefined
}

function quantityModelToForm(
  model: UsagePricing | undefined,
  unitPrice: string
): Record<string, unknown> {
  const priced = model ? protoPricingToFormData({ case: 'usagePricing', value: model }) : null
  if (!priced) return { quantityModel: 'per_unit', unitPrice }
  if (priced.pricingType === 'perUnit') {
    return { quantityModel: 'per_unit', unitPrice: priced.data.unitPrice }
  }
  return { quantityModel: priced.pricingType, unitPrice: unitPrice || '0.00', ...priced.data }
}

function dimensionRatesToForm(rates: Pick<UsagePricing_DimensionRatesPricing, 'dimensions' | 'rates' | 'unitSize' | 'onUnmatched'>) {
  return {
    dimensions: [...rates.dimensions],
    keyRates: rates.rates.map(r => ({ key: [...r.key], rate: r.rate, label: r.label })),
    unitSize: rates.unitSize || '1',
    onUnmatched: rates.onUnmatched === RateCardUnmatched.IGNORE ? 'ignore' : 'hold',
  }
}

/**
 * A matrix's cells as rates by dimension, as the server converts it: dimensions in the order they
 * first appear, `*` where a cell leaves one out, unmatched usage left unbilled.
 */
export function matrixAsRates(
  matrix: Pick<UsagePricing_MatrixPricing, 'rows'>
): Pick<UsagePricing_DimensionRatesPricing, 'dimensions' | 'rates' | 'unitSize' | 'onUnmatched'> {
  const cells = (row: UsagePricing_MatrixPricing['rows'][number]) =>
    [row.dimension1, row.dimension2].filter((d): d is NonNullable<typeof d> => !!d)
  const dimensions: string[] = []
  for (const row of matrix.rows) {
    for (const cell of cells(row)) if (!dimensions.includes(cell.key)) dimensions.push(cell.key)
  }
  return {
    dimensions,
    rates: matrix.rows.map(row =>
      create(UsagePricing_DimensionRatesPricing_DimensionRateSchema, {
        key: dimensions.map(dimension => cells(row).find(c => c.key === dimension)?.value ?? '*'),
        rate: row.perUnitPrice,
      })
    ),
    unitSize: '1',
    onUnmatched: RateCardUnmatched.IGNORE,
  }
}
