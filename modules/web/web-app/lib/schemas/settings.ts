import { z } from 'zod'

export const generalSettingsSchema = z.object({
  tradeName: z.string().min(1, 'Trade name is required'),
  tenantName: z.string().min(1, 'Tenant name is required'),
  slug: z.string().min(1, 'Tenant name is required'),
  reportingCurrency: z.string().min(1, 'Tenant name is required'),
  // Alert evaluation cadence, in seconds, as a select value. '0' = grain-derived default.
  alertEvaluationInterval: z.string(),
  keepLastEvents: z.boolean(),
})

export type GeneralSettingsSchema = z.infer<typeof generalSettingsSchema>
