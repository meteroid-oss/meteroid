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
  // Accept a standard secret key (`sk_`) or a restricted key (`rk_`). The UI
  // recommends a key scoped for webhook-endpoint creation, which in Stripe is a
  // restricted `rk_` key.
  apiSecretKey: z
    .string()
    .min(1, 'Secret key is required')
    .regex(/^(sk|rk)_/, 'Should start with sk_ or rk_'),
  // Webhook secret is now optional — leaving it blank tells the backend to
  // auto-create the endpoint via Stripe's API. Users who can't grant the
  // required scope can paste a secret manually.
  webhookSecret: z
    .string()
    .optional()
    .refine(v => !v || /^whsec_/.test(v), 'Should start with whsec_'),
})

export const hubspotIntegrationSchema = z.object({
  autoSync: z.boolean().default(true),
})

// GoCardless: bank-debit provider, mandate-based off-session charging.
// Unlike Stripe there's no auto-registration of webhook endpoints (GoCardless
// only exposes endpoint management via dashboard), so we always ask for the
// signing secret. `creditor_id` is optional — the access token usually scopes
// to a single creditor.
export const gocardlessIntegrationSchema = z.object({
  alias: z
    .string()
    .min(1, 'Name is required')
    .regex(/^[a-z0-9-]+$/, 'Only lowercase letters, numbers, and hyphens allowed'),
  accessToken: z.string().min(20, 'Access token looks too short'),
  webhookSecret: z.string().min(8, 'Webhook secret is required'),
  creditorId: z.string().optional(),
  environment: z.enum(['live', 'sandbox']).default('sandbox'),
})
