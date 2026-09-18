import { z } from 'zod'

export const hubspotIntegrationSchema = z.object({
  autoSync: z.boolean().default(true),
})
