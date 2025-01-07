import { z } from 'zod'

export const byNameSchema = z.object({
  name: z.string(),
})
export const byIdSchema = z.object({
  id: z.string(),
})
export const bySlugSchema = z.object({
  slug: z.string(),
})
export const ByLocalIdSchema = z.object({
  localId: z.string(),
})

export const paginatedCursorSchema = z.object({
  cursor: z.string(),
  pageSize: z.number().int(),
})
export const paginatedOffsetSchema = z.object({
  pageIndex: z.number().int(),
  pageSize: z.number().int(),
})
