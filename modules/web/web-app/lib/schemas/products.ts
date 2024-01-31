import { z } from 'zod'

export const createProductSchema = z.object({
  name: z.string().min(3),
  description: z.string().optional(),
})

export const createProductFamily = z.object({
  name: z.string().min(3),
  externalId: z.string().min(3),
  description: z.string().optional(),
})

export const getByExternalId = z.object({
  externalId: z.string(),
})

export const listByPlanExternalId = z.object({
  planExternalId: z.string(),
})
