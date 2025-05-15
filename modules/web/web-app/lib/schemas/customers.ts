import { z } from 'zod'

export const createCustomerSchema = z.object({
  //general
  companyName: z.string().min(3),
  alias: z.string().optional(),
  primaryEmail: z.string().optional(),
  invoicingEntity: z.string(),
  currency: z.string(),

  //Billing infos
  country: z.string().optional(),
  addressLine1: z.string().optional(),
  addressLine2: z.string().optional(),
  postalCode: z.string().optional(),
  city: z.string().optional(),
  taxId: z.string().optional(),
  shipping: z.boolean().optional(),

  //invoicing
  paymentMethod: z.string().optional(),
  stripeCustomerId: z.string().optional(),
  paymentTerm: z.number().optional(),
  gracePeriod: z.number().optional(),
  taxRate: z.number().optional(),

  //integrations
  connectorCustomerId: z.string(),
})

export type CreateCustomerSchema = z.infer<typeof createCustomerSchema>
