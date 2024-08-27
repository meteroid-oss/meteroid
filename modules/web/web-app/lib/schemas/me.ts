import { z } from 'zod'

export const emailPasswordSchema = z.object({
  email: z.string().email(),
  password: z.string().min(5, '5 characters minimum'),
})

export const registerSchema = z.object({
  email: z.string().email(),
  password: z.string().min(5, '5 characters minimum'),
})

export const accountSchema = z.object({
  firstName: z.string().min(1, 'First name is required'),
  lastName: z.string().min(1, 'Last name is required'),
  department: z.string(),
  knowUsFrom: z.string(),
})
