import { z } from 'zod'

const recipientSchema = z.object({
  name: z.string().min(1, 'Recipient name is required'),
  email: z.string().email('Valid email is required'),
})

export const createQuoteSchema = z.object({
  quote_number: z.string().min(1, 'Quote number is required'),
  customer_id: z.string().min(1, 'Customer is required'),
  plan_version_id: z.string().min(1, 'Plan is required'),
  start_date: z.date().optional(),
  billing_start_date: z.date().optional(),
  end_date: z.date().optional(),
  trial_duration: z.number().min(0).optional(),
  billing_day_anchor: z.preprocess(val => {
    if (val === '') return undefined
    return val
  }, z.number().min(1).max(31).optional()),
  expires_at: z.date().optional(),
  valid_until: z.date().optional(),
  internal_notes: z.string().optional(),
  // Unset lets the server default it: due on receipt when the quote sells credit, else the plan's.
  net_terms: z.preprocess(
    val => (val === '' || val === null ? undefined : val),
    z.number().int('Enter a whole number of days').min(0).optional()
  ),
  recipients: z.array(recipientSchema).min(1, 'At least one recipient is required'),
  activation_condition: z.enum(['ON_START', 'ON_CHECKOUT', 'MANUAL']).default('ON_START'),
  payment_methods_type: z.enum(['online', 'bankTransfer', 'external']).default('online'),
  auto_advance_invoices: z.boolean().default(true),
  charge_automatically: z.boolean().default(true),
  invoice_memo: z.string().optional(),
  create_subscription_on_acceptance: z.boolean().default(false),
  customProperties: z.record(z.string(), z.unknown()).optional(),
})

export type QuoteFormValues = z.infer<typeof createQuoteSchema>
