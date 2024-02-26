import { z } from 'zod'

export const emailPasswordSchema = z.object({
  email: z.string().email(),
  password: z.string().min(5, '5 characters minimum'),
})

export const registerSchema = z.object({
  fullName: z.string().min(1, '1 character minimum'),
  email: z.string().email(),
  password: z.string().min(5, '5 characters minimum'),
})