import { z } from 'zod'

const CURRENCIES = ['EUR', 'USD'] as const
export const balanceSchema = z.object({
  balanceValueCents: z.number().min(0, 'Must be positive'),
  balanceCurrency: z.enum(CURRENCIES, {
    errorMap: () => ({ message: "Expecting 'EUR' or 'USD'" }),
  }),
})
