import { z } from 'zod'

import { FeeTypeSchema } from '@/lib/schemas/plans'

export const createAddonSchema = z.object({
  name: z.string().min(3),
  fee: FeeTypeSchema,
})
