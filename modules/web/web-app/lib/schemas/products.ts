import { z } from 'zod'

export const createProductSchema = z.object({
  name: z.string().min(3),
  description: z.string().optional(),
})

export const createProductFamily = z.object({
  name: z.string().min(3),
  localId: z.string().min(3),
  description: z.string().optional(),
})

export const getByLocalId = z.object({
  localId: z.string(),
})

export const listByPlanLocalId = z.object({
  planLocalId: z.string(),
})
