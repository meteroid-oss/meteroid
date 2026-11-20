import { createQuoteSchema } from '@/features/quotes/editor/quoteFormSchema'

import type { QuoteFormValues } from '@/features/quotes/editor/quoteFormSchema'

export const QUOTE_SECTIONS = [
  { id: 'quote-people', label: 'Customer & people' },
  { id: 'quote-lines', label: 'Plan & lines' },
  { id: 'quote-document', label: 'Document' },
  { id: 'quote-terms', label: 'Subscription terms' },
  { id: 'quote-internal', label: 'Internal' },
] as const

export type QuoteSectionId = (typeof QUOTE_SECTIONS)[number]['id']

export interface QuoteSectionLink {
  id: QuoteSectionId
  label: string
}

/** What a template leaves out: everything specific to one deal. */
export const DEAL_FIELDS: (keyof QuoteFormValues)[] = [
  'customer_id',
  'recipients',
  'expires_at',
  'valid_until',
  'start_date',
  'billing_start_date',
  'end_date',
  'quote_number',
  'internal_notes',
  'customProperties',
]

export const TEMPLATE_SECTIONS: QuoteSectionLink[] = [
  { id: 'quote-people', label: 'Signing' },
  { id: 'quote-lines', label: 'Plan & lines' },
  { id: 'quote-document', label: 'Document' },
  { id: 'quote-terms', label: 'Subscription terms' },
]

export interface SectionIssue {
  section: QuoteSectionId
  message: string
}

const FIELDS: Record<keyof QuoteFormValues, { section: QuoteSectionId; label: string }> = {
  customer_id: { section: 'quote-people', label: 'Customer' },
  recipients: { section: 'quote-people', label: 'Recipients' },
  expires_at: { section: 'quote-people', label: 'Expiry date' },
  valid_until: { section: 'quote-people', label: 'Valid until' },
  plan_version_id: { section: 'quote-lines', label: 'Plan' },
  start_date: { section: 'quote-terms', label: 'Start date' },
  billing_start_date: { section: 'quote-terms', label: 'Billing start date' },
  end_date: { section: 'quote-terms', label: 'End date' },
  trial_duration: { section: 'quote-terms', label: 'Trial' },
  billing_day_anchor: { section: 'quote-terms', label: 'Billing day' },
  net_terms: { section: 'quote-terms', label: 'Net terms' },
  activation_condition: { section: 'quote-terms', label: 'Activation' },
  payment_methods_type: { section: 'quote-terms', label: 'Payment methods' },
  auto_advance_invoices: { section: 'quote-terms', label: 'Auto-advance invoices' },
  charge_automatically: { section: 'quote-terms', label: 'Charge automatically' },
  invoice_memo: { section: 'quote-terms', label: 'Invoice memo' },
  create_subscription_on_acceptance: { section: 'quote-terms', label: 'Subscription creation' },
  quote_number: { section: 'quote-people', label: 'Quote number' },
  internal_notes: { section: 'quote-internal', label: 'Internal notes' },
  customProperties: { section: 'quote-internal', label: 'Custom properties' },
}

// Their schema rules carry messages written for people; the others get the field's name.
const OWN_MESSAGES = new Set<string>([
  'quote_number',
  'customer_id',
  'plan_version_id',
  'recipients',
])

/** What the form's own validation refuses, by section, in the words shown on submit. */
export function formIssues(
  values: Partial<QuoteFormValues>,
  ignored: (keyof QuoteFormValues)[] = []
): SectionIssue[] {
  const parsed = createQuoteSchema.safeParse(values)
  if (parsed.success) return []

  const seen = new Set<string>()
  return parsed.error.issues.flatMap(issue => {
    const field = String(issue.path[0] ?? '') as keyof QuoteFormValues
    const meta = FIELDS[field]
    if (!meta || ignored.includes(field)) return []
    const message = OWN_MESSAGES.has(field)
      ? issue.message
      : issue.code === 'invalid_type'
        ? `${meta.label} is not a valid value`
        : `${meta.label}: ${issue.message}`
    if (seen.has(message)) return []
    seen.add(message)
    return [{ section: meta.section, message }]
  })
}
