// Audience of a payment reminder sequence: a closed leaf set mirroring the Rust
// `AudiencePredicate` (spec §1.4). Unknown keys are a deserialization error
// server-side, so nothing outside this union is ever built here.

import type { RulePredicate } from '@/features/rules/ruleModel'

export type AudienceAttributeKey =
  | 'plan_type'
  | 'invoice_kind'
  | 'payment_method_type'
  | 'currency'
  | 'country'

export type CmpOp = 'eq' | 'gt' | 'gte' | 'lt' | 'lte'
export type PropertyOp = 'in' | 'any' | CmpOp | 'set' | 'not_set'

export type AudienceLeaf =
  | { attribute: { key: AudienceAttributeKey; values: string[] } }
  | { plan: { plan_ids: string[] } }
  | { customer: { customer_ids: string[] } }
  // `threshold` in minor units of `currency`; an invoice in another currency is FALSE.
  | { amount: { op: CmpOp; threshold: number; currency: string } }
  | {
      property: {
        key: string
        op: PropertyOp
        values?: string[]
        value?: string | number | boolean | null
      }
    }
  // Reserved: rejected at save ("not supported yet").
  | { segment: { segment_id: string } }

export type AudiencePredicate = RulePredicate<AudienceLeaf>

export interface Audience {
  predicates: AudiencePredicate[]
}

export interface Option {
  value: string
  label: string
}

export const PLAN_TYPE_OPTIONS: Option[] = [
  { value: 'STANDARD', label: 'Self-serve (Standard)' },
  { value: 'FREE', label: 'Free' },
  { value: 'CUSTOM', label: 'Enterprise (Custom)' },
]

export const INVOICE_KIND_OPTIONS: Option[] = [
  { value: 'subscription', label: 'Subscription' },
  { value: 'one_off', label: 'One-off' },
  { value: 'consolidated', label: 'Consolidated' },
]

export const PAYMENT_METHOD_TYPE_OPTIONS: Option[] = [
  { value: 'CARD', label: 'Card' },
  { value: 'DIRECT_DEBIT', label: 'Direct debit' },
  { value: 'TRANSFER', label: 'Bank transfer' },
  { value: 'NONE', label: 'None' },
]

export const CMP_OP_LABEL: Record<CmpOp, string> = {
  eq: '=',
  gt: '>',
  gte: '≥',
  lt: '<',
  lte: '≤',
}

export const INVOICE_KIND_HELP =
  'Subscription: any invoice tied to a subscription, including adjustments and usage-threshold invoices. One-off: manual invoices without a subscription. Consolidated: the parent of a consolidated invoice; its children are never reminded.'

export const AUDIENCE_ROOT_HELP =
  'First matching sequence wins. Disabled sequences are skipped. The default sequence catches everything else.'
