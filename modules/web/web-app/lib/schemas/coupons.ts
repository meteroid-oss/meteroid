import { z } from 'zod'

import { parseMinorUnits } from '@/features/credit/amounts'
import { currencyPrecision } from '@/lib/data/currency-precision'

// The server's code rules (`store/repositories/coupons.rs`): 1 to 64 characters with no
// surrounding whitespace; a promotional-credit code only letters, digits, `-` and `_`.
const MAX_COUPON_CODE_LENGTH = 64
const CREDIT_CODE = /^[A-Za-z0-9_-]+$/

export const couponCodeSchema = z
  .string()
  .trim()
  .min(1, 'Enter a code')
  .refine(code => [...code].length <= MAX_COUPON_CODE_LENGTH, {
    message: `A code is at most ${MAX_COUPON_CODE_LENGTH} characters`,
  })

const baseCreateCouponSchema = z.object({
  code: couponCodeSchema,
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

// Promotional credit added to the wallet once, on redemption. Kept apart from the fixed amount:
// it is an exact decimal string, checked against the currency's minor units.
const baseCreditDiscount = z.object({
  discountType: z.literal('credit'),
  creditAmount: z.string().trim().min(1, 'Enter an amount'),
  currency: z.string().min(1, 'Select a currency'),
  creditExpiresInDays: z.preprocess(
    value => (value === '' || value === null ? undefined : value),
    z
      .number()
      .int('Enter a whole number of days')
      .min(1, 'Enter at least 1 day')
      .max(3660, 'Enter at most 3660 days')
      .optional()
  ),
  // The credit pays only the subscription the code is redeemed on, and ends with it.
  creditSubscriptionOnly: z.boolean().optional(),
})

const refineCreditDiscount = (
  data: { discountType: string; code?: string; creditAmount?: string; currency?: string },
  ctx: z.RefinementCtx
) => {
  if (data.discountType !== 'credit') return
  if (data.code && !CREDIT_CODE.test(data.code)) {
    ctx.addIssue({
      code: 'custom',
      path: ['code'],
      message: 'A promotional credit code contains only letters, digits, hyphens and underscores',
    })
  }
  if (!data.currency || !data.creditAmount) return
  const minor = parseMinorUnits(data.creditAmount, data.currency)
  if (minor === null || minor <= 0n) {
    const precision = currencyPrecision(data.currency)
    ctx.addIssue({
      code: 'custom',
      path: ['creditAmount'],
      message:
        precision === 0
          ? `Enter a positive whole amount of ${data.currency}`
          : `Enter a positive amount with at most ${precision} decimals`,
    })
  }
}

export const createCouponSchema = z
  .discriminatedUnion('discountType', [
    baseCreateCouponSchema.merge(baseFixedDiscount),
    baseCreateCouponSchema.merge(basePercentageDiscount),
    baseCreateCouponSchema.merge(baseCreditDiscount),
  ])
  .superRefine(refineCreditDiscount)

export const editComponentSchema = z
  .discriminatedUnion('discountType', [
    baseEditCouponSchema.merge(baseFixedDiscount),
    baseEditCouponSchema.merge(basePercentageDiscount),
    baseEditCouponSchema.merge(baseCreditDiscount),
  ])
  .superRefine(refineCreditDiscount)
