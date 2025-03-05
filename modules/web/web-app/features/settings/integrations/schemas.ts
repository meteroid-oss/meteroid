import { z } from 'zod'

export const stripeIntegrationSchema = z.object({
  alias: z
    .string()
    .min(1, 'Name is required')
    .regex(/^[a-z0-9-]+$/, 'Only lowercase letters, numbers, and hyphens allowed'),
  apiPublishableKey: z
    .string()
    .min(1, 'Publishable key is required')
    .regex(/^pk_/, 'Should start with pk_'),
  apiSecretKey: z.string().min(1, 'Secret key is required').regex(/^sk_/, 'Should start with sk_'),
  webhookSecret: z
    .string()
    .min(1, 'Webhook secret is required')
    .regex(/^whsec_/, 'Should start with whsec_'),
})
