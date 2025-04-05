import { z } from 'zod'

export const invitationSchema = z.object({
  email: z.array(z.string().email()),
})
