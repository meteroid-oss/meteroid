import { z } from 'zod'

export const emailPasswordSchema = z.object({
  email: z.string().email(),
  password: z.string().min(5, '5 characters minimum'),
})

export const emailSchema = z.object({
  email: z.string().email('Please enter a valid email address'),
})

export const validateEmailSchema = z
  .object({
    password: z.string().min(5, '5 characters minimum'),
    confirmPassword: z.string(),
  })
  .refine(data => data.password === data.confirmPassword, {
    message: "Passwords don't match",
    path: ['confirmPassword'],
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
