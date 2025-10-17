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
  vatNumber: z.string().optional(),
  shipping: z.boolean().optional(),
  customTaxes: z
    .array(
      z.object({
        taxCode: z.string().min(1, 'Tax code is required'),
        name: z.string().min(1, 'Tax name is required'),
        rate: z.coerce.number().min(0).max(100),
      })
    )
    .optional(),
  isTaxExempt: z.boolean().optional(),

  //invoicing
  paymentMethod: z.string().optional(),
  stripeCustomerId: z.string().optional(),
  paymentTerm: z.number().optional(),
  gracePeriod: z.number().optional(),

  //integrations
  connectorCustomerId: z.string().optional(),
})

export type CreateCustomerSchema = z.infer<typeof createCustomerSchema>
