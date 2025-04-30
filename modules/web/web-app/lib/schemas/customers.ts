import { z } from 'zod'

export const createCustomerSchema = z.object({
  //general
  companyName: z.string().min(3),
  alias: z.string().optional(),
  primaryEmail: z.string().optional(),
  invoicingEntity: z.string(),
  currency: z.string(),

  //Billing infos
  legalName: z.string(),
  country: z.string(),
  adress: z.string(),
  adressType: z.string().optional(),
  postalCode: z.string(),
  city: z.string(),
  taxId: z.string(),
  shipping: z.boolean(),

  //invoicing
  paymentMethod: z.string(),
  stripeCustomerId: z.string().optional(),
  paymentTerm: z.number(),
  gracePeriod: z.number(),
  taxRate: z.number(),

  //integrations
  connectorCustomerId: z.string(),
})

export type CreateCustomerSchema = z.infer<typeof createCustomerSchema>
