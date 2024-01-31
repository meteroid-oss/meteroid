import { z } from 'zod'

export const createTokenSchema = z.object({
  name: z.string(),
})

export const updateTokenSchema = z.object({
  id: z.string(),
  name: z.string(),
})
