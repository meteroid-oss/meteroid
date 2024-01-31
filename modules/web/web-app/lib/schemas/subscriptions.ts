import { z } from 'zod'

import { Cadence } from '@/lib/schemas/plans'

export const createSubscriptionSchema = z.object({
  customerId: z.string(),
  planVersionId: z.string(),
  planId: z.string(),
  billingStart: z.date(),
  billingEnd: z.date().optional(),
  netTerms: z.number(),
  billingDay: z.number().positive().max(31),
  parameters: z.object({
    parameters: z.array(
      z.object({
        componentId: z.string(),
        value: z.number(),
      })
    ),
    committedBillingPeriod: Cadence,
  }),
})
