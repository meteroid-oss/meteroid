import { z } from 'zod'

export const completeOnboardingSchema = z.object({
  organization: z.string().min(3),
  tenant: z.string().min(3),
  currency: z.string().length(3),
})
