import { z } from 'zod'

export const emailPasswordSchema = z.object({
  email: z.string().email(),
  password: z.string().min(8, '8 characters minimum').max(64, '64 characters maximum'),
})

export const emailSchema = z.object({
  email: z.string().email('Please enter a valid email address'),
})

export const validateEmailSchema = z
  .object({
    password: z.string().min(8, '8 characters minimum').max(64, '64 characters maximum'),
    confirmPassword: z.string(),
  })
  .refine(data => data.password === data.confirmPassword, {
    message: "Passwords don't match",
    path: ['confirmPassword'],
  })

export const registerSchema = z.object({
  email: z.string().email(),
  password: z.string().min(8, '8 characters minimum').max(64, '64 characters maximum'),
})

// Cloud asks for the department and how the user heard of us; self-hosted installations don't.
export const accountSchema = (asksAboutUser: boolean) =>
  z
    .object({
      firstName: z.string().min(1, 'First name is required'),
      lastName: z.string().min(1, 'Last name is required'),
      department: z.string().optional(),
      heardFrom: z.string().optional(),
    })
    .superRefine((data, ctx) => {
      if (!asksAboutUser) return
      if (!data.department) {
        ctx.addIssue({ code: 'custom', message: 'Required', path: ['department'] })
      }
      if (!data.heardFrom) {
        ctx.addIssue({ code: 'custom', message: 'Required', path: ['heardFrom'] })
      }
    })

export type AccountValues = z.infer<ReturnType<typeof accountSchema>>
