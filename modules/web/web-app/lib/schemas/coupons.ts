import { z } from 'zod'

const baseCreateCouponSchema = z.object({
  code: z.string().min(5),
  description: z.string().optional(),

  expiresAt: z.date().optional(),
  redemptionLimit: z.number().int().positive().optional(), // max number of subscriptions that can redeem this
  recurringValue: z.number().int().positive().optional(), // max billing periods that this coupon will be applied to
  reusable: z.boolean().optional(),
  planIds: z.array(z.string()).optional(), // restrict coupon to specific plans (empty = all plans)
})

const baseEditCouponSchema = z.object({
  description: z.string().optional(),
  planIds: z.array(z.string()).optional(),
})

const basePercentageDiscount = z.object({
  discountType: z.literal('percentage'),
  percentage: z
    .coerce
    .number()
    .positive()
    .transform(val => (val ? `${val}` : undefined)),
})

const baseFixedDiscount = z.object({
  discountType: z.literal('fixed'),
  amount: z
    .coerce
    .number()
    .positive()
    .transform(val => (val ? `${val}` : undefined)),
  currency: z.string(),
})

export const createCouponSchema = z.discriminatedUnion('discountType', [
  baseCreateCouponSchema.merge(baseFixedDiscount),
  baseCreateCouponSchema.merge(basePercentageDiscount),
])

export const editComponentSchema = z.discriminatedUnion('discountType', [
  baseEditCouponSchema.merge(baseFixedDiscount),
  baseEditCouponSchema.merge(basePercentageDiscount),
])
