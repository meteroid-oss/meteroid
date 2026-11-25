import { z } from 'zod'

import {
  CapacityPricingSchema,
  CreditPricingSchema,
  CreditUnitPricingSchema,
  DimensionRatesPricingSchema,
  PackagePricingSchema,
  PerUnitPricingSchema,
  RateCardPricingSchema,
  TieredPricingSchema,
  pricePrecision2,
  refineCreditPricing,
  refineCreditUnitPricing,
} from '@/features/pricing/schemas'

import { normalizeShape } from './compile'

import type { ChargeShape } from './compile'
import type { CreditPackContext } from '@/features/pricing/schemas'

const Cadence = z.enum(['MONTHLY', 'QUARTERLY', 'SEMIANNUAL', 'ANNUAL'])

// One flat form for every shape: the fields keep their legacy names so PricingFields binds to
// them, and the refinement checks only those of the chosen shape.
const ChargeFormBase = z.object({
  term: Cadence.default('MONTHLY'),
  metricId: z.string().optional(),
  slotUnitName: z.string().optional(),
  quantity: z.coerce.number().optional(),
  rate: z.string().optional(),
  unitPrice: z.string().optional(),
  rows: z.array(z.any()).optional(),
  packagePrice: z.string().optional(),
  blockSize: z.coerce.number().optional(),
  rateSource: z.enum(['plan', 'card']).optional(),
  rateCardId: z.string().optional(),
  markup: z.string().optional(),
  dimensions: z.array(z.string()).optional(),
  keyRates: z.array(z.any()).optional(),
  unitSize: z.string().optional(),
  onUnmatched: z.enum(['hold', 'ignore']).optional(),
  thresholds: z.array(z.any()).optional(),
  minSlots: z.coerce.number().int().positive().optional(),
  maxSlots: z.coerce.number().int().positive().optional(),
  ...CreditUnitPricingSchema.shape,
  creditAmount: z.string().optional(),
  grantCreditUnitId: z.string().default(''),
  expiryUnit: z.enum(['NONE', 'DAY', 'MONTH', 'BILLING_PERIOD']).default('NONE'),
  expiryValue: z.coerce.number().int().min(1).max(4294967295).default(1),
  rolloverEnabled: z.boolean().default(false),
  rolloverMaxAmount: z.string().default(''),
  rolloverExpiryUnit: z.enum(['DAY', 'MONTH']).default('MONTH'),
  rolloverExpiryValue: z.coerce.number().int().min(1).max(3660).default(1),
  // Mirrors the When answer of a credit pack; the compiler reads the answer.
  once: z.boolean().default(false),
})
export type ChargeFormData = z.infer<typeof ChargeFormBase>

const Quantity = z.object({
  quantity: z.coerce.number().int().positive('Enter a quantity of at least 1'),
})

function check(schema: z.ZodType, data: unknown, ctx: z.RefinementCtx) {
  const result = schema.safeParse(data)
  if (!result.success) {
    for (const issue of result.error.issues) ctx.addIssue(issue as never)
  }
}

function requireText(value: string | undefined, path: string, message: string, ctx: z.RefinementCtx) {
  if (!value?.trim()) ctx.addIssue({ code: z.ZodIssueCode.custom, path: [path], message })
}

/** Whether two capacity levels include the same amount. */
export function hasDuplicateLevels(thresholds: unknown): boolean {
  if (!Array.isArray(thresholds)) return false
  const amounts = thresholds.map(level => Number((level as { included?: unknown })?.included ?? 0))
  return new Set(amounts).size !== amounts.length
}

export interface ChargeSchemaOptions {
  creditPack?: CreditPackContext
  /** A new product names its metric and seat unit; an existing one already has them. */
  newProduct?: boolean
}

export function chargeFormSchema(shape: ChargeShape, options: ChargeSchemaOptions = {}) {
  const s = normalizeShape(shape)
  return ChargeFormBase.superRefine((data, ctx) => {
    if (s.source === 'metered') {
      if (options.newProduct) requireText(data.metricId, 'metricId', 'Choose a metric', ctx)
      refineCreditUnitPricing(data, ctx)
    }
    if (s.source === 'seats' && options.newProduct) {
      requireText(data.slotUnitName, 'slotUnitName', 'Name the unit, e.g. Seats', ctx)
    }
    if (s.source === 'fixed' && s.model !== 'flat' && s.model !== 'credit_pack') {
      check(Quantity, data, ctx)
    }
    switch (s.model) {
      case 'flat':
        check(z.object({ rate: pricePrecision2 }), data, ctx)
        break
      case 'per_unit':
        check(
          s.source === 'metered' ? PerUnitPricingSchema : z.object({ unitPrice: pricePrecision2 }),
          data,
          ctx
        )
        break
      case 'tiered':
      case 'volume':
        check(TieredPricingSchema, { rows: data.rows ?? [] }, ctx)
        break
      case 'package':
        check(PackagePricingSchema, data, ctx)
        break
      case 'by_dimension':
        if (data.rateSource === 'card') check(RateCardPricingSchema, data, ctx)
        else
          check(
            DimensionRatesPricingSchema,
            {
              dimensions: data.dimensions ?? [],
              keyRates: data.keyRates ?? [],
              unitSize: data.unitSize,
              onUnmatched: data.onUnmatched,
            },
            ctx
          )
        break
      case 'included':
        check(
          z.object({
            thresholds: z.array(CapacityPricingSchema).min(1, 'At least one level is required'),
          }),
          data,
          ctx
        )
        // The customer picks a level by what it includes: two alike could not be told apart.
        if (hasDuplicateLevels(data.thresholds)) {
          ctx.addIssue({
            code: z.ZodIssueCode.custom,
            path: ['thresholds'],
            message: 'Two levels include the same amount',
          })
        }
        break
      case 'credit_pack':
        check(CreditPricingSchema, data, ctx)
        refineCreditPricing(data, ctx, options.creditPack)
        break
    }
  })
}
