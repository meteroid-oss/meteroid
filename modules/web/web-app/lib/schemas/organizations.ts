import { z } from 'zod'

export const organizationOnboardingSchema = z.object({
  tradeName: z.string().min(1),
  country: z.string().min(2),
})
