import { isLeaf, sanitizePredicates } from '@/features/rules/ruleModel'
import { getCountryFlagEmoji } from '@/features/settings/utils'
import { formatCurrency } from '@/lib/utils/numbers'

import {
  CMP_OP_LABEL,
  INVOICE_KIND_OPTIONS,
  PAYMENT_METHOD_TYPE_OPTIONS,
  PLAN_TYPE_OPTIONS,
  type AudienceLeaf,
  type AudiencePredicate,
} from './types'

/** Stored JSON (`PaymentReminderSequence.audience`) → predicate list; absent or broken = none. */
export const parseAudience = (json?: string): AudiencePredicate[] => {
  if (!json) return []
  try {
    const parsed: unknown = JSON.parse(json)
    const predicates =
      parsed && typeof parsed === 'object' && Array.isArray((parsed as { predicates?: unknown }).predicates)
        ? ((parsed as { predicates: unknown[] }).predicates as AudiencePredicate[])
        : []
    return predicates.filter(p => p && typeof p === 'object')
  } catch {
    return []
  }
}

export const sanitizeAudience = (predicates: AudiencePredicate[]): AudiencePredicate[] =>
  sanitizePredicates<AudienceLeaf>(predicates)

/** Predicate list → the JSON the server stores; degenerate groups pruned first. */
export const serializeAudience = (predicates: AudiencePredicate[]): string =>
  JSON.stringify({ predicates: sanitizeAudience(predicates) })

/** Names resolved by the caller so labels read as the operator sees them. */
export interface AudienceLabelContext {
  planNames?: Map<string, string>
  customerNames?: Map<string, string>
  propertyNames?: Map<string, string>
}

const optionLabel = (options: { value: string; label: string }[], value: string) =>
  options.find(o => o.value === value)?.label ?? value

const listOf = (values: string[], max = 3) =>
  values.length > max ? `${values.slice(0, max).join(', ')} +${values.length - max}` : values.join(', ')

/** Names when known; ids not resolved yet (or deleted) count as "+N", never shown raw. */
const namesOf = (ids: string[], names: Map<string, string> | undefined, noun: string) => {
  const known = ids.flatMap(id => names?.get(id) ?? [])
  const unknown = ids.length - known.length
  if (known.length === 0) return `${ids.length} ${noun}${ids.length === 1 ? '' : 's'}`
  return unknown > 0 ? `${listOf(known)} +${unknown}` : listOf(known)
}

export const leafLabel = (leaf: AudienceLeaf, ctx: AudienceLabelContext = {}): string => {
  if ('attribute' in leaf) {
    const { key, values } = leaf.attribute
    switch (key) {
      case 'plan_type':
        return `Plan type in ${listOf(values.map(v => optionLabel(PLAN_TYPE_OPTIONS, v)))}`
      case 'invoice_kind':
        return `Invoice kind in ${listOf(values.map(v => optionLabel(INVOICE_KIND_OPTIONS, v)))}`
      case 'payment_method_type':
        return `Payment method in ${listOf(values.map(v => optionLabel(PAYMENT_METHOD_TYPE_OPTIONS, v)))}`
      case 'currency':
        return `Currency in ${listOf(values)}`
      case 'country':
        return `Country in ${listOf(values.map(c => `${getCountryFlagEmoji(c)} ${c}`))}`
    }
  }
  if ('plan' in leaf) {
    return `Plan in ${namesOf(leaf.plan.plan_ids, ctx.planNames, 'plan')}`
  }
  if ('customer' in leaf) {
    return `Customer in ${namesOf(leaf.customer.customer_ids, ctx.customerNames, 'customer')}`
  }
  if ('amount' in leaf) {
    const { op, threshold, currency } = leaf.amount
    return `Amount due ${CMP_OP_LABEL[op]} ${formatCurrency(threshold, currency)}`
  }
  if ('property' in leaf) {
    const { key, op, values, value } = leaf.property
    const name = ctx.propertyNames?.get(key) ?? key
    switch (op) {
      case 'in':
        return `${name} in ${listOf(values ?? [])}`
      case 'any':
        return `${name} has any of ${listOf(values ?? [])}`
      case 'set':
        return `${name} is set`
      case 'not_set':
        return `${name} is not set`
      default:
        return `${name} ${CMP_OP_LABEL[op]} ${String(value)}`
    }
  }
  return 'Saved segment (not supported yet)'
}

/** Whole predicate, groups included, as one line: the card summary and the chips share it. */
const audienceLabel = (predicate: AudiencePredicate, ctx: AudienceLabelContext = {}): string => {
  if (isLeaf(predicate)) return leafLabel(predicate, ctx)
  if ('not' in predicate) return `not (${audienceLabel(predicate.not.child, ctx)})`
  const [joiner, children] =
    'and' in predicate ? [' and ', predicate.and.children] : [' or ', predicate.or.children]
  return `(${children.map(c => audienceLabel(c, ctx)).join(joiner)})`
}

export const MAX_GROUP_DEPTH = 2

/** Nesting of and/or/not groups; the server rejects more than two levels. */
export const groupDepth = (predicate: AudiencePredicate): number => {
  if (isLeaf(predicate)) return 0
  if ('not' in predicate) return 1 + groupDepth(predicate.not.child)
  const children = 'and' in predicate ? predicate.and.children : predicate.or.children
  return 1 + Math.max(0, ...children.map(groupDepth))
}

/** Root list as one line; the root is an implicit AND. */
export const audienceSummary = (predicates: AudiencePredicate[], ctx: AudienceLabelContext = {}): string =>
  predicates.map(p => audienceLabel(p, ctx)).join(' and ')

/** Every leaf, groups flattened — for hydrating names and flagging archived properties. */
export const audienceLeaves = (predicates: AudiencePredicate[]): AudienceLeaf[] =>
  predicates.flatMap(p => {
    if (isLeaf(p)) return [p]
    if ('not' in p) return audienceLeaves([p.not.child])
    return audienceLeaves('and' in p ? p.and.children : p.or.children)
  })
