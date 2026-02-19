import { z } from 'zod'

import {
  pricePrecision2,
  pricePrecision8,
  PerUnitPricingSchema,
  TieredPricingSchema,
  VolumePricingSchema,
  PackagePricingSchema,
  MatrixPricingSchema,
  ExtraRecurringPricingSchema,
  OneTimePricingSchema,
  TierRowSchema,
} from './schemas'

const Cadence = z.enum(['MONTHLY', 'QUARTERLY', 'SEMIANNUAL', 'ANNUAL'])

// --- Single-cadence: rate, slot ---

export const RateComponentSchema = z.object({
  term: Cadence,
  rate: pricePrecision2,
})
export type RateComponentData = z.infer<typeof RateComponentSchema>

export const SlotComponentSchema = z.object({
  slotUnitName: z.string().min(1, 'Unit name is required'),
  upgradePolicy: z.enum(['PRORATED']).default('PRORATED'),
  downgradePolicy: z.enum(['REMOVE_AT_END_OF_PERIOD']).default('REMOVE_AT_END_OF_PERIOD'),
  minimumCount: z.coerce.number().int().positive().optional(),
  term: Cadence,
  unitRate: pricePrecision2,
})
export type SlotComponentData = z.infer<typeof SlotComponentSchema>

// --- Capacity with threshold table ---

const CapacityThresholdSchema = z.object({
  included: z.coerce.number().int().min(0),
  rate: pricePrecision2,
  overageRate: pricePrecision8,
})

export const CapacityComponentSchema = z.object({
  metricId: z.string().min(1, 'Metric is required'),
  term: Cadence,
  thresholds: z.array(CapacityThresholdSchema).min(1, 'At least one threshold is required'),
})
export type CapacityComponentData = z.infer<typeof CapacityComponentSchema>

// Usage: the pricing sub-schema is dynamic based on the model
export const UsagePerUnitComponentSchema = z.object({
  metricId: z.string().min(1),
  usageModel: z.literal('per_unit'),
  term: Cadence,
  ...PerUnitPricingSchema.shape,
})

export const UsageTieredComponentSchema = z.object({
  metricId: z.string().min(1),
  usageModel: z.literal('tiered'),
  term: Cadence,
  rows: z.array(TierRowSchema).min(2, 'At least 2 tiers required'),
})

export const UsageVolumeComponentSchema = z.object({
  metricId: z.string().min(1),
  usageModel: z.literal('volume'),
  term: Cadence,
  rows: z.array(TierRowSchema).min(2, 'At least 2 tiers required'),
})

export const UsagePackageComponentSchema = z.object({
  metricId: z.string().min(1),
  usageModel: z.literal('package'),
  term: Cadence,
  ...PackagePricingSchema.shape,
})

export const UsageMatrixComponentSchema = z.object({
  metricId: z.string().min(1),
  usageModel: z.literal('matrix'),
  term: Cadence,
  ...MatrixPricingSchema.shape,
})

export const UsageComponentSchema = z.discriminatedUnion('usageModel', [
  UsagePerUnitComponentSchema,
  UsageTieredComponentSchema,
  UsageVolumeComponentSchema,
  UsagePackageComponentSchema,
  UsageMatrixComponentSchema,
])
export type UsageComponentData = z.infer<typeof UsageComponentSchema>

// Flat form schema for usage — PricingFields reads field names from form root,
// so all pricing fields live at the top level. superRefine validates the active model.
export const UsageFormSchema = z
  .object({
    metricId: z.string().min(1, 'Metric is required'),
    usageModel: z.enum(['per_unit', 'tiered', 'volume', 'package', 'matrix']),
    term: Cadence,
    // per_unit
    unitPrice: z.string().optional(),
    // tiered, volume, matrix (different row shapes share this field name)
    rows: z.array(z.any()).optional(),
    // package
    packagePrice: z.string().optional(),
    blockSize: z.coerce.number().optional(),
  })
  .superRefine((data, ctx) => {
    switch (data.usageModel) {
      case 'per_unit': {
        const result = PerUnitPricingSchema.safeParse({ unitPrice: data.unitPrice })
        if (!result.success) {
          for (const issue of result.error.issues) ctx.addIssue(issue)
        }
        break
      }
      case 'tiered':
      case 'volume': {
        const schema = data.usageModel === 'tiered' ? TieredPricingSchema : VolumePricingSchema
        const result = schema.safeParse({ rows: data.rows ?? [] })
        if (!result.success) {
          for (const issue of result.error.issues) ctx.addIssue(issue)
        }
        break
      }
      case 'package': {
        const result = PackagePricingSchema.safeParse({
          packagePrice: data.packagePrice,
          blockSize: data.blockSize,
        })
        if (!result.success) {
          for (const issue of result.error.issues) ctx.addIssue(issue)
        }
        break
      }
      case 'matrix': {
        const result = MatrixPricingSchema.safeParse({ rows: data.rows ?? [] })
        if (!result.success) {
          for (const issue of result.error.issues) ctx.addIssue(issue)
        }
        break
      }
    }
  })
export type UsageFormData = z.infer<typeof UsageFormSchema>

export const ExtraRecurringComponentSchema = z.object({
  billingType: z.enum(['ADVANCE', 'ARREAR']),
  term: Cadence,
  ...ExtraRecurringPricingSchema.shape,
})
export type ExtraRecurringComponentData = z.infer<typeof ExtraRecurringComponentSchema>

// --- No-cadence types ---

export const OneTimeComponentSchema = z.object({
  ...OneTimePricingSchema.shape,
})
export type OneTimeComponentData = z.infer<typeof OneTimeComponentSchema>

// Schema lookup by fee type
export const componentSchemas = {
  rate: RateComponentSchema,
  slot: SlotComponentSchema,
  capacity: CapacityComponentSchema,
  usage: UsageComponentSchema,
  extraRecurring: ExtraRecurringComponentSchema,
  oneTime: OneTimeComponentSchema,
} as const
