import { z } from 'zod'

export const createPlanSchema = z.object({
  planName: z.string().nonempty('Name is required').max(256),
  description: z.string().max(2048),
  externalId: z
    .string()
    .nonempty('API Name is required')
    .min(3)
    .max(128)
    .regex(
      /^[a-z0-9-_]+$/,
      'Only lowercase alphanumeric characters, dashes and underscores are allowed'
    ),
  planType: z.enum(['FREE', 'STANDARD', 'CUSTOM']).default('STANDARD'),
})

const isValidNumber = (str: string) => {
  const replacedStr = str.replace(',', '.')
  return !isNaN(parseFloat(replacedStr)) && isFinite(parseFloat(replacedStr))
}

const isPreciseString = (str: string, precision: number) => {
  const replacedStr = str.replace(',', '.')
  const parts = replacedStr.split('.')
  return parts.length < 2 || parts[1].length <= precision
}

const precisionValidation = (str: string, precision: number) => {
  return isValidNumber(str) && isPreciseString(str, precision)
}

const pricePrecision2Schema = z.string().refine(price => precisionValidation(price, 2), {
  message: 'Price can have a maximum of 2 decimal places',
  path: [],
})

// For 8 decimal places
const pricePrecision8Schema = z.string().refine(price => precisionValidation(price, 8), {
  message: 'Price can have a maximum of 8 decimal places',
  path: [],
})

const BillingType = z.enum(['ARREAR', 'ADVANCE'])
export type BillingType = z.infer<typeof BillingType>

const BlockSizeSchema = z.object({
  blockSize: z.number().positive().int(),
})
export type BlockSize = z.infer<typeof BlockSizeSchema>

const FixedFeePricingSchema = z.object({
  unitPrice: pricePrecision2Schema,
  quantity: z.number().positive().int(),
  billingType: BillingType,
})
export type FixedFeePricing = z.infer<typeof FixedFeePricingSchema>

// PerUnit Schema
const PerUnitSchema = z.object({
  unitPrice: pricePrecision8Schema,
})
export type PerUnit = z.infer<typeof PerUnitSchema>

// TieredAndVolume Row Schema
const TieredAndVolumeRowSchema = z
  .object({
    firstUnit: z.number().nonnegative().int(),
    lastUnit: z.number().nonnegative().int().optional(),
    unitPrice: pricePrecision8Schema,
    flatFee: pricePrecision2Schema.optional(),
    flatCap: pricePrecision2Schema.optional(),
  })
  .refine(data => data.firstUnit < (data.lastUnit ?? Number.MAX_VALUE), {
    message: 'First unit must be less than last unit',
    path: ['lastUnit'],
  })
export type TieredAndVolumeRow = z.infer<typeof TieredAndVolumeRowSchema>

// TieredAndVolume Schema
const TieredAndVolumeSchema = z.object({
  rows: z.array(TieredAndVolumeRowSchema),
  blockSize: BlockSizeSchema.optional(),
})
export type TieredAndVolume = z.infer<typeof TieredAndVolumeSchema>

// Package Schema
const PackageSchema = z.object({
  blockSize: z.number().positive().int(),
  blockPrice: pricePrecision8Schema,
})
export type Package = z.infer<typeof PackageSchema>

// Usage Pricing Model Schema
const UsagePricingModelSchema = z.discriminatedUnion('model', [
  z.object({ model: z.literal('per_unit'), data: PerUnitSchema }),
  z.object({ model: z.literal('tiered'), data: TieredAndVolumeSchema }),
  z.object({ model: z.literal('volume'), data: TieredAndVolumeSchema }),
  z.object({ model: z.literal('package'), data: PackageSchema }),
])
export type UsagePricingModel = z.infer<typeof UsagePricingModelSchema>

export type UsagePricingModelType = UsagePricingModel['model']

// Cadence Enum
export const Cadence = z.enum([
  'MONTHLY',
  'QUARTERLY',
  /*'SEMI_ANNUAL',*/ 'ANNUAL' /*'BIENNIAL', 'TRIENNIAL'*/,
])
export type Cadence = z.infer<typeof Cadence>

export const OneTimeFeeSchema = z.object({
  pricing: FixedFeePricingSchema,
})
export type OneTimeFee = z.infer<typeof OneTimeFeeSchema>

export const RecurringFixedFeeSchema = z.object({
  fee: FixedFeePricingSchema,
  cadence: Cadence,
})
export type RecurringFixedFee = z.infer<typeof RecurringFixedFeeSchema>

const SingleTermSchema = z.object({
  price: pricePrecision2Schema,
  cadence: Cadence,
})
export type SingleTerm = z.infer<typeof SingleTermSchema>

const TermRateSchema = z.object({
  term: Cadence,
  price: pricePrecision2Schema,
})
export type TermRate = z.infer<typeof TermRateSchema>

const TermBasedSchema = z.object({
  rates: z.array(TermRateSchema),
})
export type TermBased = z.infer<typeof TermBasedSchema>

export const TermFeePricingSchema = z.union([SingleTermSchema, TermBasedSchema])
export type TermFeePricing = z.infer<typeof TermFeePricingSchema>

export const SubscriptionRateSchema = z.object({
  pricing: TermFeePricingSchema,
})
export type SubscriptionRate = z.infer<typeof SubscriptionRateSchema>

const SlotUnitSchema = z.object({
  id: z.string().uuid().optional(),
  name: z.string(),
})
export type SlotUnit = z.infer<typeof SlotUnitSchema>

export const SlotBasedSchema = z.object({
  pricing: TermFeePricingSchema,
  slotUnit: SlotUnitSchema,
  upgradePolicy: z.enum(['PRORATED']),
  downgradePolicy: z.enum(['REMOVE_AT_END_OF_PERIOD']),
  minimumCount: z.number().positive().int().optional(),
  quota: z.number().positive().int().optional(),
})
export type SlotBased = z.infer<typeof SlotBasedSchema>

const BillableMetricSchema = z.object({
  id: z.string().uuid(),
  name: z.string().optional(),
})
export type BillableMetric = z.infer<typeof BillableMetricSchema>

const ThresholdSchema = z.object({
  includedAmount: z.number().nonnegative(),
  price: pricePrecision2Schema,
  perUnitOverage: pricePrecision8Schema,
})
export type Threshold = z.infer<typeof ThresholdSchema>

const SingleTermCapacitySchema = z.object({
  thresholds: z.array(ThresholdSchema),
})
export type SingleTermCapacity = z.infer<typeof SingleTermCapacitySchema>

const TermBasedCapacitySchema = z.object({
  rates: z.array(
    z.object({
      term: Cadence,
      thresholds: z.array(ThresholdSchema),
    })
  ),
})
export type TermBasedCapacity = z.infer<typeof TermBasedCapacitySchema>

const CapacityPricingSchema = z.union([SingleTermCapacitySchema, TermBasedCapacitySchema])
export type CapacityPricing = z.infer<typeof CapacityPricingSchema>

export const CapacitySchema = z.object({
  metric: BillableMetricSchema,
  pricing: CapacityPricingSchema,
})
export type Capacity = z.infer<typeof CapacitySchema>

export const UsageBasedSchema = z.object({
  metric: BillableMetricSchema,
  model: UsagePricingModelSchema,
})
export type UsageBased = z.infer<typeof UsageBasedSchema>

const FeeTypeSchema = z.discriminatedUnion('fee', [
  z.object({ fee: z.literal('rate'), data: SubscriptionRateSchema }),
  z.object({ fee: z.literal('slot_based'), data: SlotBasedSchema }),
  z.object({ fee: z.literal('capacity'), data: CapacitySchema }),
  z.object({ fee: z.literal('usage_based'), data: UsageBasedSchema }),
  z.object({ fee: z.literal('recurring'), data: RecurringFixedFeeSchema }),
  z.object({ fee: z.literal('one_time'), data: OneTimeFeeSchema }),
])
export type FeeType = z.infer<typeof FeeTypeSchema>

// type R = z.ZodSchema<FeeType['data']>
// const x: R = SubscriptionRateSchema

export const PriceComponentSchema = z.object({
  id: z.string(),
  name: z.string(),
  fee: FeeTypeSchema,
  productItem: z
    .object({
      id: z.string(),
      name: z.string(),
    })
    .optional(),
})
export type PriceComponent = z.infer<typeof PriceComponentSchema>

export const byPlanVersionSchema = z.object({
  externalId: z.string(),
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
  productItemId: z.string().optional(),
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
