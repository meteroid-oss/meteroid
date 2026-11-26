import { decodeDrafts, encodeDrafts } from '@/features/entitlements/rows/grantDraft'
import { emptyQuoteV2State, withLegacyDocument } from '@/features/quotes/editor/quoteV2'
import { emptyPriceComponentsState } from '@/features/quotes/quoteFormSeed'

import type { QuoteFormValues } from '@/features/quotes/editor/quoteFormSchema'
import type { QuoteV2State } from '@/features/quotes/editor/quoteV2'
import type { QuoteFormSeed } from '@/features/quotes/quoteFormSeed'

type LegacyFormValues = Partial<QuoteFormValues> & {
  overview?: string
  terms_and_services?: string
}

export const DRAFT_PAYLOAD_VERSION = 1

function replacer(this: Record<string, unknown>, key: string, value: unknown) {
  // JSON.stringify has already turned a Date into a string by the time `value` arrives.
  const raw = this[key]
  if (raw instanceof Date) return Number.isNaN(raw.getTime()) ? null : { $date: raw.toISOString() }
  if (typeof raw === 'bigint') return { $bigint: raw.toString() }
  return value
}

const reviver = (_key: string, value: unknown) => {
  if (value && typeof value === 'object' && !Array.isArray(value)) {
    const keys = Object.keys(value)
    if (keys.length === 1) {
      const tagged = value as { $date?: unknown; $bigint?: unknown }
      if (typeof tagged.$date === 'string') return new Date(tagged.$date)
      if (typeof tagged.$bigint === 'string') return BigInt(tagged.$bigint)
    }
  }
  return value
}

// Payloads written before a field existed lack it: each one takes the editor's default.
const decodeV2 = (v2: Partial<QuoteV2State> | undefined): QuoteV2State => {
  const defaults = emptyQuoteV2State()
  return {
    ...defaults,
    ...v2,
    displayOptions: { ...defaults.displayOptions, ...v2?.displayOptions },
    customerActions: { ...defaults.customerActions, ...v2?.customerActions },
  }
}

// Entitlement drafts hold protobuf messages: they're stored as their proto JSON.
export const encodeSnapshot = (snapshot: QuoteFormSeed): string =>
  JSON.stringify({ ...snapshot, pendingEntitlements: encodeDrafts(snapshot.pendingEntitlements) }, replacer)

/** A snapshot from a payload of any age: what it lacks takes the editor's defaults. */
export function decodeSnapshot(payload: string): QuoteFormSeed | undefined {
  try {
    const parsed = JSON.parse(payload, reviver) as Partial<QuoteFormSeed> | null
    if (!parsed || typeof parsed !== 'object' || !parsed.values) return undefined
    // Payloads from before sections carried the overview and terms as form fields.
    const { overview, terms_and_services: terms, ...values } = parsed.values as LegacyFormValues
    const v2 = decodeV2(parsed.v2)
    return {
      values,
      priceComponents: parsed.priceComponents ?? emptyPriceComponentsState(),
      addOns: parsed.addOns ?? [],
      coupons: parsed.coupons ?? [],
      minimumCommitment: parsed.minimumCommitment,
      pendingEntitlements: decodeDrafts(parsed.pendingEntitlements),
      v2: { ...v2, sections: withLegacyDocument(v2.sections, overview, terms) },
      resourceId: parsed.resourceId,
    }
  } catch {
    return undefined
  }
}

/** The editor state as a template: nothing specific to one deal (customer, recipients, number, dates). */
export function toTemplatePayload(snapshot: QuoteFormSeed): unknown {
  const {
    customer_id: _customer,
    recipients: _recipients,
    quote_number: _number,
    start_date: _start,
    billing_start_date: _billingStart,
    end_date: _end,
    expires_at: _expires,
    valid_until: _valid,
    ...values
  } = snapshot.values
  const template: QuoteFormSeed = {
    ...snapshot,
    values,
    v2: { ...snapshot.v2, templateId: undefined, hubspotDealId: '' },
    resourceId: undefined,
  }
  return JSON.parse(encodeSnapshot(template))
}

export const fromTemplatePayload = (payload: unknown): QuoteFormSeed | undefined =>
  decodeSnapshot(typeof payload === 'string' ? payload : JSON.stringify(payload))
