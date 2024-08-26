import { z } from 'zod'

export const organizationOnboardingSchema = z.object({
  tradeName: z.string().min(1),
  country: z.string().min(2),
  legalName: z.string().optional(),
  vatNumber: z.string().optional(),
  addressLine1: z.string().optional(),
  addressLine2: z.string().optional(),
  zipCode: z.string().optional(),
  state: z.string().optional(),
  city: z.string().optional(),
})
