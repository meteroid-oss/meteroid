import { z } from 'zod'

import { FeeType , FeeStructure_UsageModel } from '@/rpc/api/prices/v1/models_pb'

const isValidNumber = (str: string) => !isNaN(parseFloat(str)) && isFinite(parseFloat(str))
const isPrecise = (str: string, precision: number) => {
  const parts = str.split('.')
  return parts.length < 2 || parts[1].length <= precision
}

export const pricePrecision2 = z.string().refine(
  v => isValidNumber(v) && isPrecise(v, 2),
  { message: 'Must be a valid price (max 2 decimal places)' }
)

export const pricePrecision8 = z.string().refine(
  v => isValidNumber(v) && isPrecise(v, 8),
  { message: 'Must be a valid price (max 8 decimal places)' }
)

// Rate — flat rate per billing period
export const RatePricingSchema = z.object({
  rate: pricePrecision2,
})
export type RatePricingData = z.infer<typeof RatePricingSchema>

// Slot — price per unit/seat
export const SlotPricingSchema = z.object({
  unitRate: pricePrecision2,
  minSlots: z.coerce.number().int().positive().optional(),
  maxSlots: z.coerce.number().int().positive().optional(),
})
export type SlotPricingData = z.infer<typeof SlotPricingSchema>

// Capacity — committed capacity with overage
export const CapacityPricingSchema = z.object({
  rate: pricePrecision2,
  included: z.coerce.number().int().min(0),
  overageRate: pricePrecision8,
})
export type CapacityPricingData = z.infer<typeof CapacityPricingSchema>

// Usage - Tier row (shared between tiered and volume)
export const TierRowSchema = z.object({
  firstUnit: z.coerce.bigint().nonnegative(),
  unitPrice: pricePrecision8,
  flatFee: pricePrecision2.optional(),
  flatCap: pricePrecision2.optional(),
})

// Usage - Per Unit
export const PerUnitPricingSchema = z.object({
  unitPrice: pricePrecision8,
})
export type PerUnitPricingData = z.infer<typeof PerUnitPricingSchema>

// Usage - Tiered
export const TieredPricingSchema = z
  .object({
    rows: z.array(TierRowSchema).min(2, 'At least 2 tiers required'),
  })
  .superRefine((data, ctx) => {
    if (data.rows.length > 0 && data.rows[0].firstUnit !== BigInt(0)) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: 'First tier must start at 0',
        path: ['rows', 0, 'firstUnit'],
      })
    }
    for (let i = 1; i < data.rows.length; i++) {
      if (data.rows[i].firstUnit <= data.rows[i - 1].firstUnit) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          message: 'Tiers must be in ascending order',
          path: ['rows', i, 'firstUnit'],
        })
      }
    }
  })
export type TieredPricingData = z.infer<typeof TieredPricingSchema>

// Usage - Volume (same shape as tiered)
export const VolumePricingSchema = TieredPricingSchema
export type VolumePricingData = z.infer<typeof VolumePricingSchema>

// Usage - Package
export const PackagePricingSchema = z.object({
  packagePrice: pricePrecision8,
  blockSize: z.coerce.number().int().min(1, 'Block size must be at least 1'),
})
export type PackagePricingData = z.infer<typeof PackagePricingSchema>

// Usage - Matrix dimension
export const MatrixDimensionSchema = z.object({
  key: z.string(),
  value: z.string(),
})

export const MatrixRowSchema = z.object({
  perUnitPrice: pricePrecision8,
  dimension1: MatrixDimensionSchema,
  dimension2: MatrixDimensionSchema.optional(),
})

export const MatrixPricingSchema = z.object({
  rows: z.array(MatrixRowSchema).min(1, 'At least 1 matrix row required'),
})
export type MatrixPricingData = z.infer<typeof MatrixPricingSchema>

// Extra Recurring
export const ExtraRecurringPricingSchema = z.object({
  unitPrice: pricePrecision2,
  quantity: z.coerce.number().int().positive(),
})
export type ExtraRecurringPricingData = z.infer<typeof ExtraRecurringPricingSchema>

// One-Time
export const OneTimePricingSchema = z.object({
  unitPrice: pricePrecision2,
  quantity: z.coerce.number().int().positive(),
})
export type OneTimePricingData = z.infer<typeof OneTimePricingSchema>

// Pricing type discriminator
export type PricingType =
  | 'rate'
  | 'slot'
  | 'capacity'
  | 'perUnit'
  | 'tiered'
  | 'volume'
  | 'package'
  | 'matrix'
  | 'extraRecurring'
  | 'oneTime'

// Schema lookup
export const pricingSchemas: Record<PricingType, z.ZodType> = {
  rate: RatePricingSchema,
  slot: SlotPricingSchema,
  capacity: CapacityPricingSchema,
  perUnit: PerUnitPricingSchema,
  tiered: TieredPricingSchema,
  volume: VolumePricingSchema,
  package: PackagePricingSchema,
  matrix: MatrixPricingSchema,
  extraRecurring: ExtraRecurringPricingSchema,
  oneTime: OneTimePricingSchema,
}

// Map proto enums to PricingType
export function toPricingType(
  feeType: FeeType,
  usageModel?: FeeStructure_UsageModel
): PricingType {
  switch (feeType) {
    case FeeType.RATE:
      return 'rate'
    case FeeType.SLOT:
      return 'slot'
    case FeeType.CAPACITY:
      return 'capacity'
    case FeeType.USAGE:
      switch (usageModel) {
        case FeeStructure_UsageModel.PER_UNIT:
          return 'perUnit'
        case FeeStructure_UsageModel.TIERED:
          return 'tiered'
        case FeeStructure_UsageModel.VOLUME:
          return 'volume'
        case FeeStructure_UsageModel.PACKAGE:
          return 'package'
        case FeeStructure_UsageModel.MATRIX:
          return 'matrix'
        default:
          return 'perUnit'
      }
    case FeeType.EXTRA_RECURRING:
      return 'extraRecurring'
    case FeeType.ONE_TIME:
      return 'oneTime'
  }
}

// Default values per pricing type
export function pricingDefaults(pricingType: PricingType): Record<string, unknown> {
  switch (pricingType) {
    case 'rate':
      return { rate: '0.00' }
    case 'slot':
      return { unitRate: '0.00' }
    case 'capacity':
      return { rate: '0.00', included: 0, overageRate: '0.00000000' }
    case 'perUnit':
      return { unitPrice: '0.00000000' }
    case 'tiered':
    case 'volume':
      return {
        rows: [
          { firstUnit: BigInt(0), unitPrice: '' },
          { firstUnit: BigInt(100), unitPrice: '' },
        ],
      }
    case 'package':
      return { packagePrice: '0.00000000', blockSize: 1 }
    case 'matrix':
      return { rows: [] }
    case 'extraRecurring':
      return { unitPrice: '0.00', quantity: 1 }
    case 'oneTime':
      return { unitPrice: '0.00', quantity: 1 }
  }
}
