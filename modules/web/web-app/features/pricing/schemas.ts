import Decimal from 'decimal.js'
import { z } from 'zod'

import { currencyPrecision } from '@/lib/data/currency-precision'
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

// Credit is checked against its unit's own precision (0 to 6) by the server.
export const creditPrecision6 = z.string().refine(
  v => isValidNumber(v) && isPrecise(v, 6),
  { message: 'Must be a valid amount (max 6 decimal places)' }
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

/**
 * Usage (or capacity overage) priced in a credit unit: its amounts are credits, and credits the
 * customer's wallet does not cover are billed at `creditUnitRate` (price currency per credit).
 * An empty `creditUnitId` prices in the plan currency. An empty rate is allowed where access is
 * denied at zero (`creditRateOptional`: it bills nothing) or where the unit has a value in the plan
 * currency (`creditUnitValued`: the server bills the overage at that value).
 */
export const CreditUnitPricingSchema = z.object({
  creditUnitId: z.string().default(''),
  creditUnitRate: z.string().default(''),
  creditRateOptional: z.boolean().default(false),
  creditUnitValued: z.boolean().default(false),
})
export type CreditUnitPricingData = z.infer<typeof CreditUnitPricingSchema>

export function refineCreditUnitPricing(
  data: Partial<CreditUnitPricingData>,
  ctx: z.RefinementCtx
) {
  if (!data.creditUnitId) return
  const rate = data.creditUnitRate?.trim() ?? ''
  const optional = !!data.creditRateOptional || !!data.creditUnitValued
  const valid = optional && !rate ? true : Number(rate) > 0
  if (!valid) {
    ctx.addIssue({
      code: z.ZodIssueCode.custom,
      path: ['creditUnitRate'],
      message: optional
        ? 'The overage price must be positive, or empty'
        : 'Enter an overage price per credit, or give the credit unit a value in this currency',
    })
  }
}

/** A pack priced at zero is credit included in the plan. */
export const isZeroAmount = (value: unknown) => /^0*(?:\.0*)?$/.test(String(value ?? ''))

// Only a free pack with an expiry, granted every period, rolls over: elsewhere the switch is
// disabled and the rollover is not sent.
export const packRollsOver = (
  d: Partial<{ rolloverEnabled: boolean; once: boolean; unitPrice: string; expiryUnit: string }>
) => !!d.rolloverEnabled && !d.once && isZeroAmount(d.unitPrice) && d.expiryUnit !== 'NONE'

// Credit pack: charged at unitPrice, funds creditAmount of invoice credit per pack, in the plan
// currency or, when `grantCreditUnitId` is set, in that credit unit.
export const CreditPricingSchema = z.object({
  unitPrice: pricePrecision2,
  quantity: z.coerce.number().int().positive(),
  creditAmount: creditPrecision6.refine(value => Number(value) > 0, 'Credit must be positive'),
  grantCreditUnitId: z.string().default(''),
  expiryUnit: z.enum(['NONE', 'DAY', 'MONTH', 'BILLING_PERIOD']).default('NONE'),
  expiryValue: z.coerce.number().int().min(1).max(4294967295).default(1),
  // Included packs that expire only: unspent credit is carried once into a new grant.
  rolloverEnabled: z.boolean().default(false),
  rolloverMaxAmount: z
    .string()
    .default('')
    .refine(value => value.trim() === '' || Number(value) > 0, 'The cap must be positive'),
  rolloverExpiryUnit: z.enum(['DAY', 'MONTH']).default('MONTH'),
  rolloverExpiryValue: z.coerce.number().int().min(1).max(3660).default(1),
  // Granted on the subscription's first invoice only; such a grant never rolls over.
  once: z.boolean().default(false),
})
export type CreditPricingData = z.infer<typeof CreditPricingSchema>

/** What a credit pack's credit is counted in: its unit's precision, else the plan currency's. */
export type CreditPackContext = {
  currency: string
  unitPrecision: (creditUnitId: string) => number | undefined
}

const MAX_ROLLOVER_MONTHS = 120

const toDecimal = (value: string | undefined): Decimal | undefined => {
  if (!value?.trim()) return undefined
  try {
    return new Decimal(value.trim())
  } catch {
    return undefined
  }
}

const precisionMessage = (precision: number) =>
  precision === 0 ? 'Enter a whole number' : `Enter at most ${precision} decimals`

/** The server's credit pack rules (`store/domain/prices.rs`), so a refusal shows on its field. */
export function refineCreditPricing(
  data: Partial<CreditPricingData>,
  ctx: z.RefinementCtx,
  context?: CreditPackContext
) {
  if (data.once && data.rolloverEnabled) {
    ctx.addIssue({
      code: z.ZodIssueCode.custom,
      path: ['once'],
      message: 'Credit granted once at start cannot roll over',
    })
  }
  if (
    packRollsOver(data) &&
    data.rolloverExpiryUnit === 'MONTH' &&
    (data.rolloverExpiryValue ?? 0) > MAX_ROLLOVER_MONTHS
  ) {
    ctx.addIssue({
      code: z.ZodIssueCode.custom,
      path: ['rolloverExpiryValue'],
      message: `Enter at most ${MAX_ROLLOVER_MONTHS} months`,
    })
  }
  const unitId = data.grantCreditUnitId ?? ''
  const price = toDecimal(data.unitPrice)
  const credit = toDecimal(data.creditAmount)
  // Money credit sold below its price would be paid credit worth more than its face.
  if (!unitId && price && credit && price.gt(credit)) {
    ctx.addIssue({
      code: z.ZodIssueCode.custom,
      path: ['creditAmount'],
      message: 'A pack cannot cost more than the credit it grants',
    })
  }
  if (!context) return
  const precision = unitId ? context.unitPrecision(unitId) : currencyPrecision(context.currency)
  if (precision === undefined) return
  for (const field of ['creditAmount', 'rolloverMaxAmount'] as const) {
    if (field === 'rolloverMaxAmount' && !packRollsOver(data)) continue
    const places = toDecimal(data[field])?.decimalPlaces()
    if (places !== undefined && places > precision) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        path: [field],
        message: precisionMessage(precision),
      })
    }
  }
}

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
  | 'credit'

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
  credit: CreditPricingSchema.superRefine(refineCreditPricing),
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
    case FeeType.CREDIT:
      return 'credit'
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
    case 'credit':
      return {
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
