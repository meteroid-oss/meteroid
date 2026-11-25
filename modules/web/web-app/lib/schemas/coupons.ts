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

// The billing periods starting within this many months of the first paid period.
const durationMonthsField = z.preprocess(
  value => (value === '' || value === null ? undefined : value),
  z
    .number()
    .int('Enter a whole number of months')
    .min(1, 'Enter at least 1 month')
    .max(120, 'Enter at most 120 months')
    .optional()
)

const baseCreateCouponSchema = z.object({
  code: couponCodeSchema,
  description: z.string().optional(),

  validFrom: z.date().optional(),
  expiresAt: z.date().optional(),
  // max number of subscriptions that can redeem this; a cleared input sends ''
  redemptionLimit: z.preprocess(
    value => (value === '' || value === null ? undefined : value),
    z.coerce.number().int().positive().optional()
  ),
  // Once (the first invoice), forever, or the periods starting within N months of the first
  // paid period.
  duration: z.enum(['once', 'forever', 'months']),
  durationMonths: durationMonthsField,
  reusable: z.boolean().optional(),
  planIds: z.array(z.string()).optional(), // restrict coupon to specific plans (empty = all plans)
  addOnIds: z.array(z.string()).optional(), // with planIds empty too: everything
})

const baseEditCouponSchema = z.object({
  description: z.string().optional(),
  planIds: z.array(z.string()).optional(),
  addOnIds: z.array(z.string()).optional(),
  validFrom: z.date().optional(),
  expiresAt: z.date().optional(),
  // Unset on a coupon that counts invoices (made before months) or gives credit: kept as it is.
  duration: z.enum(['forever', 'months']).optional(),
  durationMonths: durationMonthsField,
})

const basePercentageDiscount = z.object({
  discountType: z.literal('percentage'),
  percentage: z
    .coerce
    .number()
    .positive()
    .transform(val => (val ? `${val}` : undefined)),
})

// The same discount in another currency it is redeemable in, one row per currency.
const fixedAmountRow = z.object({
  currency: z.string().min(1, 'Select a currency'),
  amount: z
    .string()
    .trim()
    .refine(val => Number(val) > 0, { message: 'Enter a positive amount' }),
})

const baseFixedDiscount = z.object({
  discountType: z.literal('fixed'),
  amount: z
    .coerce
    .number()
    .positive()
    .transform(val => (val ? `${val}` : undefined)),
  currency: z.string(),
  otherAmounts: z.array(fixedAmountRow).optional(),
})

const refineOtherAmounts = (
  data: { discountType: string; currency?: string; otherAmounts?: { currency: string }[] },
  ctx: z.RefinementCtx
) => {
  if (data.discountType !== 'fixed') return
  const currencies = [data.currency, ...(data.otherAmounts ?? []).map(row => row.currency)]
  currencies.forEach((currency, index) => {
    if (index > 0 && currencies.indexOf(currency) !== index) {
      ctx.addIssue({
        code: 'custom',
        path: ['otherAmounts', index - 1, 'currency'],
        message: `${currency} already has an amount`,
      })
    }
  })
}

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

const refineDuration = (
  data: { discountType: string; duration?: string; durationMonths?: number },
  ctx: z.RefinementCtx
) => {
  const lastsMonths = data.discountType !== 'credit' && data.duration === 'months'
  if (lastsMonths && data.durationMonths === undefined) {
    ctx.addIssue({ code: 'custom', path: ['durationMonths'], message: 'Enter a number of months' })
  }
}

const refineValidity = (
  data: { validFrom?: Date; expiresAt?: Date },
  ctx: z.RefinementCtx
) => {
  if (data.validFrom && data.expiresAt && data.expiresAt <= data.validFrom) {
    ctx.addIssue({
      code: 'custom',
      path: ['expiresAt'],
      message: 'Ends after it starts',
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
  .superRefine(refineOtherAmounts)
  .superRefine(refineDuration)
  .superRefine(refineValidity)

export const editComponentSchema = z
  .discriminatedUnion('discountType', [
    baseEditCouponSchema.merge(baseFixedDiscount),
    baseEditCouponSchema.merge(basePercentageDiscount),
    baseEditCouponSchema.merge(baseCreditDiscount),
  ])
  .superRefine(refineCreditDiscount)
  .superRefine(refineOtherAmounts)
  .superRefine(refineDuration)
  .superRefine(refineValidity)
