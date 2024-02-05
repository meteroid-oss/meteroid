import { z } from 'zod'

const addressSchema = z.object({
  line1: z.string().optional(),
  line2: z.string().optional(),
  city: z.string().optional(),
  country: z.string().optional(),
  state: z.string().optional(),
  zipcode: z.string().optional(),
})
const shippingAddressSchema = z.object({
  address: addressSchema.optional(),
  sameThanBilling: z.boolean(),
})
export const addressesSchema = z.object({
  billing_address: addressSchema.optional(),
  shipping_address: shippingAddressSchema.optional(),
})
