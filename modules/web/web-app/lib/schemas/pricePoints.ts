import { z } from 'zod'

const durationSchema = z.object({
  value: z.number().int().positive(),
  unit: z.enum(['DAY', 'MONTH']),
})

const trialPriceRampSchema = z.object({
  name: z.string().nonempty('Name is required').max(256),
  duration: durationSchema,
  netTerms: z.number().nonnegative().max(180),
})

const pricedProduct = z.object({
  productId: z.string(),
  discount: z.any().optional(),
  minimum: z.any().optional(),
  customPricingUnitId: z.string().optional(),
  recurringFees: z.array(z.any()),
  usageBasedFees: z.array(z.any()),
})
export type PricedProductSchema = z.infer<typeof pricedProduct>

/*

        name: string;
  durationInBillingPeriods?: number | undefined;
  discount: ProductDiscount | undefined;
  minimum: AmountMinimum | undefined;
  freeCreditAmount?: number | undefined;
  idx: number;

  */

const priceRampSchema = z.object({
  name: z.string().nonempty('Name is required').max(256),
  durationInBillingPeriods: z.number().positive().optional(),
  discount: z.any().optional(),
  minimum: z.any().optional(),
  freeCreditAmount: z.number().optional(),
  idx: z.number(),
})
export type PriceRampSchema = z.infer<typeof priceRampSchema>

export const createPricePointSchema = z.object({
  pricePointName: z.string().nonempty('Name is required').max(256),
  planLocalId: z.string(),
  currency: z.string(),
  cycle: z.enum(['FOREVER', 'FIXED']),
  frequency: z.enum(['ANNUAL', 'MONTHLY', 'QUARTERLY', 'SEMI_ANNUAL', 'SEMI_MONTHLY']),
  periodStart: z.enum(['DAY_OF_MONTH', 'START_OF_PLAN']),
  trial: trialPriceRampSchema.optional(),
  phases: z.array(priceRampSchema), // TODO refine, last duration is empty of cycle is forever
  products: z.array(pricedProduct),
})

export const listPricePointsSchema = z.object({
  planLocalId: z.string(),
})
