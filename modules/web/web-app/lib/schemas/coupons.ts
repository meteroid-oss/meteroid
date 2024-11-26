import { z } from 'zod'

export const createCouponschema = z
  .object({
    code: z.string().min(5),
    description: z.string().optional(),
    // discount
    discountType: z.enum(['percentage', 'fixed']),
    percentage: z.string().optional(),
    amount: z.string().optional(),
    currency: z.string().optional(),
    //
    expiresAt: z.date().optional(),
    redemptionLimit: z.number().int().optional(), // max number of subscriptions that can redeem this
    recurringValue: z.number().int().optional(), // max billing periods that this coupon will be applied to
    reusable: z.boolean().optional(),
  })
  .superRefine((data, ctx) => {
    if (data.discountType === 'percentage' && !data.percentage) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        path: ['percentage'],
        message: `Percentage is required.`,
      })
    }
    if (data.discountType === 'fixed') {
      if (!data.amount) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          path: ['amount'],
          message: `Amount is required.`,
        })
      }
      if (!data.currency) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          path: ['currency'],
          message: `Currency is required.`,
        })
      }
    }
  })

export const editComponentSchema = z
  .object({
    description: z.string().optional(),
    // discount
    discountType: z.enum(['percentage', 'fixed']),
    percentage: z.string().optional(),
    amount: z.string().optional(),
    currency: z.string().optional(),
  })
  .superRefine((data, ctx) => {
    if (data.discountType === 'percentage' && !data.percentage) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        path: ['percentage'],
        message: `Percentage is required.`,
      })
    }
    if (data.discountType === 'fixed') {
      if (!data.amount) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          path: ['amount'],
          message: `Amount is required.`,
        })
      }
      if (!data.currency) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          path: ['currency'],
          message: `Currency is required.`,
        })
      }
    }
  })
