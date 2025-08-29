import { z } from 'zod'

const addressSchema = z.object({
  line1: z.string().optional(),
  line2: z.string().optional(),
  city: z.string().optional(),
  country: z.string().optional(),
  state: z.string().optional(),
  zipCode: z.string().optional(),
})

const shippingAddressSchema = z.object({
  address: addressSchema.optional(),
  sameAsBilling: z.boolean().default(false),
})

export const customerSchema = z.object({
  name: z.string().min(3, 'Required'),
  alias: z.string().optional(),
  email: z.string().email().optional(),
  invoicingEmail: z.string().email().optional(),
  phone: z.string().optional(),
  vatNumber: z.string().optional(),
  customTaxRate: z.string().optional(),
  isTaxExempt: z.boolean().default(false),
  billingAddress: addressSchema.optional(),
  shippingAddress: shippingAddressSchema.optional(),
})
