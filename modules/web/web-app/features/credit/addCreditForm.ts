import { z } from 'zod'

import { productScopeSchema, refineProductScope } from '@/features/credit/ProductScopeField'
import { amountSchema } from '@/features/credit/amounts'
import { quantitySchema, unitPriceSchema } from '@/features/credit/unitSale'

import type { CreditUnitLabel } from '@/rpc/api/wallets/v1/models_pb'

export const DEFAULT_PRIORITY = 100
const MAX_EXPIRY_MONTHS = 120

/** Months-from-now text input, valid blank (never expires) or an integer in range. */
const monthsValid = (months: string) => {
  const trimmed = months.trim()
  return (
    !trimmed ||
    (/^\d+$/.test(trimmed) && Number(trimmed) >= 1 && Number(trimmed) <= MAX_EXPIRY_MONTHS)
  )
}

/** The products the subscription a credit is restricted to bills, once known. */
export type ScopeContext = { subscriptionProductIds: ReadonlySet<string>; planName: string }

export const buildSchema = (
  currency: string,
  creditUnit?: CreditUnitLabel,
  readScope: () => ScopeContext | undefined = () => undefined
) =>
  z
    .object({
      // Never preselected: a goodwill credit lowers the taxed amount, a payment does not.
      type: z.enum(['promotional', 'outside', 'paid'], { error: 'Choose what this credit is' }),
      collection: z.enum(['charge', 'invoice']),
      amount: creditUnit ? quantitySchema(creditUnit.precision) : amountSchema(currency),
      unitPrice: z.string(),
      paymentMethodId: z.string(),
      // Same control (months from now) for every kind of credit added here.
      expiresInMonths: z.string(),
      priority: z.number().int().min(0, 'Priority cannot be negative'),
      reason: z.string().trim().max(1024),
      reference: z.string().trim().max(255),
      note: z.string().trim().max(1024),
      productScope: productScopeSchema,
      // Null: any of the customer's subscriptions can use the credit.
      subscriptionId: z.string().nullable(),
      appliesTo: z.enum(['ALL', 'USAGE']),
    })
    .superRefine((values, ctx) => {
      if (values.type === 'outside') {
        if (!values.reason) {
          ctx.addIssue({ code: 'custom', path: ['reason'], message: 'Say what the payment was' })
        }
        return
      }
      if (values.type === 'promotional') {
        if (!values.reason) {
          ctx.addIssue({ code: 'custom', path: ['reason'], message: 'Say why the credit is added' })
        }
        refineProductScope(values.productScope, ctx)
        const scope = values.subscriptionId ? readScope() : undefined
        if (
          scope &&
          values.productScope.mode === 'selected' &&
          values.productScope.productIds.some(id => !scope.subscriptionProductIds.has(id))
        ) {
          ctx.addIssue({
            code: 'custom',
            path: ['productScope', 'productIds'],
            message: `${scope.planName} does not bill every selected product: credit for it would never pay the others.`,
          })
        }
        if (!monthsValid(values.expiresInMonths)) {
          ctx.addIssue({
            code: 'custom',
            path: ['expiresInMonths'],
            message: `Enter a number of months between 1 and ${MAX_EXPIRY_MONTHS}`,
          })
        }
        return
      }
      if (creditUnit) {
        const price = unitPriceSchema.safeParse(values.unitPrice)
        if (!price.success) {
          ctx.addIssue({
            code: 'custom',
            path: ['unitPrice'],
            message: price.error.issues[0]?.message,
          })
        }
      }
      if (values.collection === 'charge' && !values.paymentMethodId) {
        ctx.addIssue({
          code: 'custom',
          path: ['paymentMethodId'],
          message: 'Choose the payment method to charge',
        })
      }
      if (!monthsValid(values.expiresInMonths)) {
        ctx.addIssue({
          code: 'custom',
          path: ['expiresInMonths'],
          message: `Enter a number of months between 1 and ${MAX_EXPIRY_MONTHS}`,
        })
      }
    })

export type Values = z.infer<ReturnType<typeof buildSchema>>

/** `undefined` for a blank input: the credit never expires. The server dates it, in UTC. */
export const monthsOrNever = (months: string): number | undefined =>
  months.trim() ? Number(months.trim()) : undefined

export type CreditNature = Values['type']

/** Submitting needs the nature chosen: it decides whether the credit comes off before tax. */
export const canSubmitCredit = (nature: CreditNature | undefined, pending: boolean) =>
  !!nature && !pending
