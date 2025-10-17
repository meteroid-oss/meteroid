import { z } from 'zod'

export const createPlanSchema = z.object({
  planName: z.string().min(1, 'Name is required').max(256),
  description: z.string().max(2048).optional(),
  productFamilyLocalId: z.string().min(1, 'A product family is required'),
  planType: z.enum(['FREE', 'STANDARD', 'CUSTOM']).default('STANDARD'),
  currency: z.string().min(1, 'Currency is required'),
})

export const editPlanSchema = z.object({
  planName: z.string().nonempty('Name is required').max(256),
  description: z.string().max(2048).optional(),
  netTerms: z.number().int(),
})

const isValidNumber = (str: string) => {
  return !isNaN(parseFloat(str)) && isFinite(parseFloat(str))
}

const isPreciseString = (str: string, precision: number) => {
  const parts = str.split('.')
  return parts.length < 2 || parts[1].length <= precision
}

const precisionValidation = (str: string, precision: number) => {
  return isValidNumber(str) && isPreciseString(str, precision)
}

const pricePrecision2Schema = z.string().refine(price => precisionValidation(price, 2), {
  message: 'Price must be defined and have a maximum of 2 decimal places',
  path: [],
})

// For 8 decimal places
const pricePrecision8Schema = z.string().refine(price => precisionValidation(price, 8), {
  message: 'Price must be defined and have a maximum of 8 decimal places',
  path: [],
})

const BillingType = z.enum(['ARREAR', 'ADVANCE'])
export type BillingType = z.infer<typeof BillingType>

export const Cadence = z.enum([
  'MONTHLY',
  'QUARTERLY',
  'SEMIANNUAL',
  'ANNUAL' /*'BIENNIAL', 'TRIENNIAL'*/,
])
export type Cadence = z.infer<typeof Cadence>

const TermRateSchema = z.object({
  term: Cadence,
  price: pricePrecision2Schema,
})
export type TermRate = z.infer<typeof TermRateSchema>

export const RateFeeSchema = z.object({
  rates: z.array(TermRateSchema).min(1, 'At least one rate is required'),
})
export type RateFee = z.infer<typeof RateFeeSchema>

export const SlotFeeSchema = z.object({
  rates: z.array(TermRateSchema).min(1, 'At least one rate is required'),
  slotUnitName: z.string(),
  upgradePolicy: z.enum(['PRORATED']),
  downgradePolicy: z.enum(['REMOVE_AT_END_OF_PERIOD']),
  minimumCount: z.number().positive().int().optional(),
  quota: z.number().positive().int().optional(),
})
export type SlotFee = z.infer<typeof SlotFeeSchema>

const BillableMetricSchema = z.object({
  id: z.string(),
  name: z.string().optional(),
})
export type BillableMetric = z.infer<typeof BillableMetricSchema>

const CapacityThresholdSchema = z.object({
  includedAmount: z.coerce.bigint().nonnegative(),
  price: pricePrecision2Schema,
  perUnitOverage: pricePrecision8Schema,
})
export type CapacityThreshold = z.infer<typeof CapacityThresholdSchema>

export const CapacityFeeSchema = z.object({
  metricId: z.string(),
  thresholds: z.array(CapacityThresholdSchema),
})
export type CapacityFee = z.infer<typeof CapacityFeeSchema>

const TieredAndVolumeRowSchema = z.object({
  firstUnit: z.bigint().nonnegative(),
  unitPrice: pricePrecision8Schema,
  flatFee: pricePrecision2Schema.optional(),
  flatCap: pricePrecision2Schema.optional(),
})
export type TieredAndVolumeRow = z.infer<typeof TieredAndVolumeRowSchema>

const TieredAndVolumeSchema = z
  .object({
    rows: z.array(TieredAndVolumeRowSchema),
    blockSize: z.coerce.bigint().positive().optional(),
  })
  .superRefine((data, ctx) => {
    if (data.rows[0].firstUnit !== BigInt(0)) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: 'First unit of first row must be zero',
        path: ['rows', 0, 'firstUnit'],
      })
    }
    for (let i = 1; i < data.rows.length; i++) {
      if (data.rows[i].firstUnit <= data.rows[i - 1].firstUnit) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          message: 'First unit must be higher than the previous row',
          path: ['rows', i, 'firstUnit'],
        })
      }
    }
  })

export type TieredAndVolume = z.infer<typeof TieredAndVolumeSchema>

const PerUnitSchema = z.object({
  unitPrice: pricePrecision8Schema,
})
export type PerUnit = z.infer<typeof PerUnitSchema>

// Package Schema
const PackageSchema = z.object({
  blockSize: z.coerce.bigint().positive(),
  packagePrice: pricePrecision8Schema,
})
export type Package = z.infer<typeof PackageSchema>

const dimensionSchema = z.object({
  key: z.string(),
  value: z.string(),
})

export type Dimension = z.infer<typeof dimensionSchema>

const MatrixSchema = z.object({
  dimensionRates: z
    .array(
      z.object({
        dimension1: dimensionSchema,
        dimension2: dimensionSchema.optional(),
        price: pricePrecision2Schema,
      })
    )
    .min(1),
})

export type Matrix = z.infer<typeof MatrixSchema>

const UsagePricingModelSchema = z.discriminatedUnion('model', [
  z.object({ model: z.literal('per_unit'), data: PerUnitSchema }),
  z.object({
    model: z.literal('volume'),
    data: TieredAndVolumeSchema,
  }),
  z.object({
    model: z.literal('tiered'),
    data: TieredAndVolumeSchema,
  }),
  z.object({ model: z.literal('package'), data: PackageSchema }),
  z.object({ model: z.literal('matrix'), data: MatrixSchema }),
])

export type UsagePricingModel = z.infer<typeof UsagePricingModelSchema>
export type UsagePricingModelType = UsagePricingModel['model']

export const UsageFeeSchema = z.object({
  metricId: z.string(),
  model: UsagePricingModelSchema,
})
export type UsageFee = z.infer<typeof UsageFeeSchema>

export const ExtraRecurringFeeSchema = z.object({
  unitPrice: pricePrecision2Schema,
  quantity: z.number().positive().int(),
  billingType: BillingType,
  term: Cadence.optional(),
})
export type ExtraRecurringFee = z.infer<typeof ExtraRecurringFeeSchema>

export const OneTimeFeeSchema = z.object({
  unitPrice: pricePrecision2Schema,
  quantity: z.number().positive().int(),
})
export type OneTimeFee = z.infer<typeof OneTimeFeeSchema>

export const FeeTypeSchema = z.discriminatedUnion('fee', [
  z.object({ fee: z.literal('rate'), data: RateFeeSchema }),
  z.object({ fee: z.literal('slot'), data: SlotFeeSchema }),
  z.object({ fee: z.literal('capacity'), data: CapacityFeeSchema }),
  z.object({ fee: z.literal('usage'), data: UsageFeeSchema }),
  z.object({ fee: z.literal('extraRecurring'), data: ExtraRecurringFeeSchema }),
  z.object({ fee: z.literal('oneTime'), data: OneTimeFeeSchema }),
])
export type FeeType = z.infer<typeof FeeTypeSchema>

export const PriceComponentSchema = z.object({
  id: z.string(),
  name: z.string(),
  localId: z.string(),
  fee: FeeTypeSchema,
  productId: z.string().optional(),
})
export type PriceComponent = z.infer<typeof PriceComponentSchema>

export const byPlanVersionSchema = z.object({
  localId: z.string(),
  version: z.number().int().optional(),
})

export const byPlanVersionIdSchema = z.object({
  planId: z.string(),
  planVersionId: z.string(),
})
export const byPlanIdSchema = z.object({
  planId: z.string(),
})

export const addPriceComponentSchema = z.object({
  planVersionId: z.string(),
  name: z.string(),
  fee: FeeTypeSchema,
  productId: z.string().optional(),
})
export type AddPriceComponent = z.infer<typeof addPriceComponentSchema>

export const formPriceCompoentSchema = z.object({
  name: z.string(),
  fee: FeeTypeSchema,
})
export type FormPriceComponent = z.infer<typeof formPriceCompoentSchema>

export const editPriceComponentSchema = z.object({
  id: z.string(),
  planVersionId: z.string(),
  name: z.string(),
  fee: FeeTypeSchema,
})

export const draftPlanOverviewSchema = z.object({
  planVersionId: z.string(),
  planId: z.string(),
  name: z.string(),
  description: z.string().optional(),
  currency: z.string(),
  netTerms: z.number().int(),
  billingPeriods: z.array(Cadence),
})

export const publishedPlanOverviewSchema = z.object({
  planVersionId: z.string(),
  planId: z.string(),
  name: z.string(),
  description: z.string().optional(),
})

export const trialConfigSchema = z.object({
  durationDays: z.number().int(),
  downgradePlanId: z.string().optional(),
  trialingPlanId: z.string().optional(),
  requiresPreAuthorization: z.boolean().optional(),
  actionAfterTrial: z.enum(['BLOCK', 'CHARGE', 'DOWNGRADE']).optional(),
})
export type TrialConfigSchema = z.infer<typeof trialConfigSchema>
