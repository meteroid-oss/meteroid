import { z } from 'zod'

export const customerSchema = z.object({
  name: z.string().min(3, 'Required'),
  alias: z.string().optional(),
  email: z.string().optional(),
  invoicingEmail: z.string().optional(),
  phone: z.string().optional(),
})
