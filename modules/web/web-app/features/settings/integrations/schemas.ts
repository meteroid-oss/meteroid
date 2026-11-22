import { z } from 'zod'

export const hubspotIntegrationSchema = z.object({
  autoSync: z.boolean().default(true),
})

export const ALL_INVOICING_ENTITIES = 'all'

export const pennylaneIntegrationSchema = z.object({
  invoicingEntityId: z.string(),
})
