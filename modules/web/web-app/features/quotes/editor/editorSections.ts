import { createQuoteSchema } from '@/features/quotes/editor/quoteFormSchema'

import type { QuoteFormValues } from '@/features/quotes/editor/quoteFormSchema'

/** The editor's steps: what the quote is, what goes with it, and who signs it. */
export const QUOTE_STEPS = [
  { id: 1, label: 'Quote details' },
  { id: 2, label: 'Document & payment' },
  { id: 3, label: 'Review & send' },
] as const

export type QuoteStep = (typeof QUOTE_STEPS)[number]['id']

export const QUOTE_SECTIONS = [
  { id: 'quote-details', label: 'Quote details', step: 1 },
  { id: 'quote-lines', label: 'Line items', step: 1 },
  { id: 'quote-terms', label: 'Subscription terms', step: 1 },
  { id: 'quote-document', label: 'Document', step: 2 },
  { id: 'quote-payment', label: 'Payment & invoicing', step: 2 },
  { id: 'quote-internal', label: 'Internal', step: 2 },
  { id: 'quote-summary', label: 'Summary', step: 3 },
  { id: 'quote-signers', label: 'Who signs', step: 3 },
] as const

export type QuoteSectionId = (typeof QUOTE_SECTIONS)[number]['id']

export interface QuoteSectionLink {
  id: QuoteSectionId
  label: string
  step: QuoteStep
}

export const stepOfSection = (id: QuoteSectionId): QuoteStep =>
  QUOTE_SECTIONS.find(section => section.id === id)?.step ?? 1

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
  { id: 'quote-details', label: 'Plan', step: 1 },
  { id: 'quote-lines', label: 'Line items', step: 1 },
  { id: 'quote-terms', label: 'Subscription terms', step: 1 },
  { id: 'quote-document', label: 'Document', step: 2 },
  { id: 'quote-payment', label: 'Payment & invoicing', step: 2 },
  { id: 'quote-signers', label: 'Signing', step: 3 },
]

export interface SectionIssue {
  section: QuoteSectionId
  message: string
}

const FIELDS: Record<keyof QuoteFormValues, { section: QuoteSectionId; label: string }> = {
  customer_id: { section: 'quote-details', label: 'Customer' },
  quote_number: { section: 'quote-details', label: 'Quote number' },
  expires_at: { section: 'quote-details', label: 'Expiry date' },
  valid_until: { section: 'quote-details', label: 'Valid until' },
  plan_version_id: { section: 'quote-details', label: 'Plan' },
  start_date: { section: 'quote-terms', label: 'Start date' },
  billing_start_date: { section: 'quote-terms', label: 'Billing start date' },
  end_date: { section: 'quote-terms', label: 'End date' },
  trial_duration: { section: 'quote-terms', label: 'Trial' },
  trial_plan_id: { section: 'quote-terms', label: 'Trial' },
  trial_end_action: { section: 'quote-terms', label: 'Trial' },
  billing_day_anchor: { section: 'quote-terms', label: 'Billing day' },
  activation_condition: { section: 'quote-terms', label: 'Activation' },
  create_subscription_on_acceptance: { section: 'quote-terms', label: 'Subscription creation' },
  net_terms: { section: 'quote-payment', label: 'Net terms' },
  payment_methods_type: { section: 'quote-payment', label: 'Payment methods' },
  auto_advance_invoices: { section: 'quote-payment', label: 'Auto-advance invoices' },
  charge_automatically: { section: 'quote-payment', label: 'Charge automatically' },
  invoice_memo: { section: 'quote-payment', label: 'Invoice memo' },
  internal_notes: { section: 'quote-internal', label: 'Internal notes' },
  customProperties: { section: 'quote-internal', label: 'Custom properties' },
  recipients: { section: 'quote-signers', label: 'Recipients' },
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
