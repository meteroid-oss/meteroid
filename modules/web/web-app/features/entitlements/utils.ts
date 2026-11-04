import { type MessageInitShape } from '@bufbuild/protobuf'
import Decimal from 'decimal.js'

import { type MetricUnit, unitNoun } from '@/features/billing/metrics/units'
import {
  CalendarUnit,
  ConfigValue,
  ConfigValueType,
  EffectiveEntitlement,
  EntitlementValue,
  EntitlementValueSchema,
  FeatureType,
  OverageBehavior,
  ResetPeriod,
  ResolvedEntitlement,
  ResolvedOrigin,
} from '@/rpc/api/entitlements/v1/models_pb'

import type { ConfigKind } from '@/features/entitlements/creation/types'

// Product grouping helper shared across entitlement views

export type ProductGroup<T> = { id: string | null; name: string; items: T[] }

export function groupByProduct<T>(
  items: T[],
  getProduct: (item: T) => { id: string; name: string } | undefined,
): ProductGroup<T>[] {
  const byId = new Map<string | null, ProductGroup<T>>()
  for (const it of items) {
    const p = getProduct(it)
    const key = p?.id ?? null
    let group = byId.get(key)
    if (!group) {
      group = { id: p?.id ?? null, name: p?.name ?? 'Global', items: [] }
      byId.set(key, group)
    }
    group.items.push(it)
  }
  return Array.from(byId.values()).sort((a, b) => {
    if (a.id === null) return 1
    if (b.id === null) return -1
    return a.name.localeCompare(b.name)
  })
}

/** Feature-code rules, mirrored by the backend's `validate_feature_code`. Shared by every code-entry form. */
export const FEATURE_CODE_REGEX = /^[a-z0-9][a-z0-9_-]*$/
export const FEATURE_CODE_MAX_LENGTH = 128
export const FEATURE_CODE_CHARSET_MESSAGE = "Lowercase letters, digits, '-', '_' (e.g. audit_log)"
export const FEATURE_CODE_LENGTH_MESSAGE = `Must be ${FEATURE_CODE_MAX_LENGTH} characters or fewer`

/** Derive a slug code from a feature name, matching the backend's allowed charset `^[a-z0-9][a-z0-9_-]*$`. */
export const slugifyCode = (name: string): string =>
  name
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, '_')
    .replace(/^[_-]+/, '')

// Local domain types — use these in UI, map from proto at the boundary

export type BooleanFeatureKind = { type: 'boolean' }
export type MeteredFeatureKind = { type: 'metered'; metricId: string }
export type ConfigFeatureKind = { type: 'config'; valueType: ConfigKind; options?: string[] }
export type FeatureKind = BooleanFeatureKind | MeteredFeatureKind | ConfigFeatureKind

export function configValueTypeToKind(vt: ConfigValueType): ConfigKind {
  switch (vt) {
    case ConfigValueType.NUMBER:
      return 'number'
    case ConfigValueType.BOOLEAN:
      return 'boolean'
    case ConfigValueType.MAP:
      return 'map'
    case ConfigValueType.JSON:
      return 'json'
    case ConfigValueType.SELECT:
      return 'select'
    case ConfigValueType.TEXT:
    case ConfigValueType.UNSPECIFIED:
    default:
      return 'text'
  }
}

export function featureKindFromProto(ft: FeatureType | undefined): FeatureKind {
  if (ft?.Inner?.case === 'metered') {
    return { type: 'metered', metricId: ft.Inner.value.metricId }
  }
  if (ft?.Inner?.case === 'config') {
    const options = ft.Inner.value.options
    return {
      type: 'config',
      valueType: configValueTypeToKind(ft.Inner.value.valueType),
      options: options.length > 0 ? options : undefined,
    }
  }
  return { type: 'boolean' }
}

export function featureTypeLabel(kind: FeatureKind): string {
  switch (kind.type) {
    case 'metered':
      return 'Metered'
    case 'config':
      return 'Config'
    default:
      return 'Boolean'
  }
}

export function configKindLabel(kind: ConfigKind): string {
  switch (kind) {
    case 'number':
      return 'Number'
    case 'boolean':
      return 'Boolean'
    case 'map':
      return 'Map (key/value)'
    case 'json':
      return 'JSON'
    case 'select':
      return 'Select (list)'
    case 'text':
    default:
      return 'Text'
  }
}

export function calendarUnitLabel(unit: CalendarUnit): string {
  switch (unit) {
    case CalendarUnit.HOUR:
      return 'hour'
    case CalendarUnit.DAY:
      return 'day'
    case CalendarUnit.WEEK:
      return 'week'
    case CalendarUnit.MONTH:
      return 'month'
    case CalendarUnit.YEAR:
      return 'year'
  }
}

export function resetPeriodLabel(rp: ResetPeriod | undefined): string {
  if (!rp) return '—'
  const { Inner } = rp
  if (!Inner || Inner.case === undefined) return '—'
  switch (Inner.case) {
    case 'billingCycle':
      return 'Billing cycle'
    case 'never':
      return 'Never'
    case 'calendar':
      return `Every ${Inner.value.interval} ${calendarUnitLabel(Inner.value.unit)}${Inner.value.interval > 1 ? 's' : ''} (calendar)`
    case 'fixedWindow':
      return `Every ${Inner.value.interval} ${calendarUnitLabel(Inner.value.unit)}${Inner.value.interval > 1 ? 's' : ''} (fixed window)`
    case 'slidingWindow':
      return `Last ${Inner.value.interval} ${calendarUnitLabel(Inner.value.unit)}${Inner.value.interval > 1 ? 's' : ''} (sliding window)`
  }
}

/** Compact one-line preview of a `json` config value — renders a flat object as `{ k: v, … }`. */
function formatConfigJson(raw: string): string {
  let parsed: unknown
  try {
    parsed = JSON.parse(raw)
  } catch {
    return raw
  }
  if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) return raw
  const entries = Object.entries(parsed)
  if (entries.length === 0) return '{ }'
  const shown = entries.slice(0, 3).map(([k, val]) => `${k}: ${val}`).join(', ')
  return entries.length > 3 ? `{ ${shown}, +${entries.length - 3} }` : `{ ${shown} }`
}

export function configValueLabel(cv: ConfigValue | undefined): string {
  const v = cv?.value
  switch (v?.case) {
    case 'number':
    case 'text':
      return v.value
    case 'json':
      return formatConfigJson(v.value)
    case 'boolean':
      return v.value ? 'true' : 'false'
    default:
      return '—'
  }
}

export function entitlementValueLabel(value: EntitlementValue['value'] | undefined): string {
  if (value?.case === 'booleanValue') {
    return value.value.enabled ? '✓ Enabled' : '✗ Disabled'
  }
  if (value?.case === 'meteredValue') {
    const m = value.value
    const limitPart = m.limit ? Number(m.limit).toLocaleString() : 'Unlimited'
    const periodPart =
      m.resetPeriod?.Inner?.case === 'billingCycle'
        ? ' per billing period'
        : m.resetPeriod
          ? ` · ${resetPeriodLabel(m.resetPeriod)}`
          : ''
    const groupPart = m.groupBy ? ` per ${m.groupBy}` : ''
    const terms = limitTermsOf(m)
    return [`${limitPart}${groupPart}${periodPart}`, limitBehaviorLabel(terms), limitAlertsLabel(terms)]
      .filter(Boolean)
      .join(' · ')
  }
  if (value?.case === 'configValue') {
    return configValueLabel(value.value)
  }
  return '—'
}

/** "5,150,000 / 20,000,000", the limit alone, or "Enabled": an effective entitlement's value. */
export function effectiveValueLabel(value: EffectiveEntitlement['value']): string {
  if (value.case === 'boolean') return value.value.enabled ? 'Enabled' : 'Disabled'
  if (value.case !== 'metered') return '—'
  const consumed = value.value.consumed ? Number(value.value.consumed) : undefined
  const limit = value.value.limit ? Number(value.value.limit) : undefined
  if (consumed !== undefined)
    return `${consumed.toLocaleString()} / ${limit !== undefined ? limit.toLocaleString() : '∞'}`
  return limit !== undefined ? limit.toLocaleString() : 'Unlimited'
}

// ── Resolved-entitlements helpers ─────────────────────────────────────────────

/**
 * True when the entitlement's kill-switch is engaged (boolean disabled or metered
 * `enabled = false`). Used to dim rows and gate "Enable/Disable" menu items.
 */
export function isEntitlementDisabled(
  value: ResolvedEntitlement['value'] | EffectiveEntitlement['value']
): boolean {
  return !configuredEnabled(value)
}

/**
 * Sentence used in the "inherited" tooltip. Branches:
 * - Origin is a feature row with a known product → "Inherited from product X."
 * - Origin is a feature row without a product → "Inherited from a global feature."
 * - Origin is anything else with a known name → "Inherited from {type} {name}."
 * - Origin missing → "Inherited."
 */
export function buildInheritanceTooltip(
  origin: ResolvedOrigin | undefined,
  featureProduct: { name: string } | undefined
): string {
  if (!origin) return 'Inherited.'
  if (origin.entity?.EntityId?.case === 'customerId') {
    return 'Forced at the customer level — overrides plan and subscription entitlements.'
  }
  const originIsFeature = origin.entity?.EntityId?.case === 'featureId'
  if (originIsFeature && featureProduct) {
    return `Inherited from product ${featureProduct.name}.`
  }
  if (originIsFeature) {
    return 'Inherited from a global feature.'
  }
  return `Inherited from ${originTypeWord(origin)} ${originLabel(origin)}.`
}

/**
 * Human-readable label for the origin of a resolved entitlement. Returns the
 * server-provided name (e.g. "Starter v3", "Extra Seats"). Falls back to a
 * type-based label if the name is missing (e.g. empty string from older data).
 */
export function originLabel(origin: ResolvedOrigin): string {
  if (origin.name) return origin.name
  // Fallback: derive a generic label from the entity type.
  switch (origin.entity?.EntityId?.case) {
    case 'featureId':
      return 'Feature default'
    case 'planId':
      return 'Plan'
    case 'planVersionId':
      return 'Plan version'
    case 'addOnId':
      return 'Add-on'
    case 'subscriptionId':
      return 'Subscription'
    case 'quoteId':
      return 'Quote'
    case 'customerId':
      return 'Customer'
    default:
      return 'Unknown'
  }
}

/** Lowercase entity type word for use in sentences like "Inherited from plan X". */
export function originTypeWord(origin: ResolvedOrigin): string {
  switch (origin.entity?.EntityId?.case) {
    case 'featureId':
      return 'feature'
    case 'planId':
      return 'plan'
    case 'planVersionId':
      return 'plan version'
    case 'addOnId':
      return 'add-on'
    case 'subscriptionId':
      return 'subscription'
    case 'quoteId':
      return 'quote'
    case 'customerId':
      return 'customer'
    default:
      return ''
  }
}

/** Groups a decimal string ("5000000" → "5,000,000"); returns it unchanged when it is not a number. */
export function formatDecimalGrouped(raw: string): string {
  let amount: Decimal
  try {
    amount = new Decimal(raw)
  } catch {
    return raw
  }
  if (amount.isInteger()) return new Intl.NumberFormat().format(BigInt(amount.toFixed(0)))
  return new Intl.NumberFormat(undefined, {
    maximumFractionDigits: amount.decimalPlaces(),
  }).format(amount.toNumber())
}

function periodSpan(interval: number, unit: CalendarUnit): string {
  const label = calendarUnitLabel(unit)
  return interval === 1 ? label : `${interval} ${label}s`
}

/** The "per period" suffix of a metered limit: " / month", " / billing period", " / rolling 7 days". */
export function resetPeriodSuffix(rp: ResetPeriod | undefined): string {
  const inner = rp?.Inner
  switch (inner?.case) {
    case 'billingCycle':
      return ' / billing period'
    case 'calendar':
      return ` / ${periodSpan(inner.value.interval, inner.value.unit)}`
    case 'fixedWindow':
      return ` / ${periodSpan(inner.value.interval, inner.value.unit)} from activation`
    case 'slidingWindow':
      return ` / rolling ${periodSpan(inner.value.interval, inner.value.unit)}`
    default:
      return ''
  }
}

/** A metered limit, in raw metric units: "5,000,000 tokens", or the bare amount without a unit. */
export function formatMeteredAmount(raw: string, unit?: MetricUnit): string {
  const amount = formatDecimalGrouped(raw)
  return unit ? `${amount} ${unitNoun(Number(raw), unit)}` : amount
}

/** Compact display string for a resolved entitlement value, e.g. "5,000,000 tokens / month". */
export function formatResolvedValue(value: ResolvedEntitlement['value'], unit?: MetricUnit): string {
  if (value.case === 'boolean') {
    return value.value.enabled ? 'Enabled' : 'Disabled'
  }
  if (value.case === 'metered') {
    const m = value.value
    if (!m.limit) return 'Unlimited'
    return `${formatMeteredAmount(m.limit, unit)}${resetPeriodSuffix(m.resetPeriod)}`
  }
  if (value.case === 'config') {
    const config = value.value.value
    const inner = config?.value
    if (inner?.case === 'number') return formatDecimalGrouped(inner.value)
    return configValueLabel(config)
  }
  return '—'
}

/** How a metered limit behaves, from a stored value or a form. */
export type LimitTerms = {
  limit?: string
  groupBy?: string
  overage: 'block' | 'allow'
  gracePct?: number
  thresholds?: number[]
}

export const limitTermsOf = (m: {
  limit?: string
  groupBy?: string
  overageBehavior?: OverageBehavior
  thresholds?: number[]
}): LimitTerms => {
  const inner = m.overageBehavior?.Inner
  return {
    limit: m.limit,
    groupBy: m.groupBy,
    overage: inner?.case === 'allow' ? 'allow' : 'block',
    gracePct: inner?.case === 'block' ? inner.value.gracePeriodPct : undefined,
    thresholds: m.thresholds,
  }
}

/** What a limit does once reached, e.g. "Hard limit (+10% grace)"; none without a limit. */
export function limitBehaviorLabel(t: LimitTerms): string | undefined {
  if (!t.limit) return undefined
  if (t.overage === 'allow') return 'Soft limit'
  return t.gracePct ? `Hard limit (+${t.gracePct}% grace)` : 'Hard limit'
}

export const limitBehaviorHelp = (t: LimitTerms): string | undefined => {
  if (!t.limit) return undefined
  return t.overage === 'allow'
    ? 'Usage continues past the limit; the overage is billed by a price on the same metric.'
    : 'Access checks return denied once the limit, plus the grace if set, is reached.'
}

/** "Alerts at 50%, 80%" for the thresholds set below 100%, which always alerts. */
export const limitAlertsLabel = (t: LimitTerms): string | undefined => {
  const below = (t.thresholds ?? []).filter(pct => pct < 100)
  return t.limit && below.length ? `Alerts at ${below.map(pct => `${pct}%`).join(', ')}` : undefined
}

/** "per user_id" when the limit applies to each value of a property separately. */
export const limitGroupLabel = (t: LimitTerms): string | undefined =>
  t.limit && t.groupBy ? `per ${t.groupBy}` : undefined

/**
 * The entitlement's own switch: an effective entitlement reads disabled once a blocking limit is
 * reached, though it is still switched on.
 */
export function configuredEnabled(
  value: ResolvedEntitlement['value'] | EffectiveEntitlement['value']
): boolean {
  if (value.case === 'boolean') return value.value.enabled
  if (value.case === 'metered') {
    const m = value.value
    return m.enabled || ('limitReached' in m && m.limitReached)
  }
  return true
}

/** A hard limit of an effective entitlement is reached: access checks deny it until it resets. */
export const isLimitReached = (
  value: ResolvedEntitlement['value'] | EffectiveEntitlement['value']
): boolean => value.case === 'metered' && 'limitReached' in value.value && value.value.limitReached

type StoredMeteredTerms = {
  limit?: string
  resetPeriod?: ResetPeriod
  groupBy?: string
  overageBehavior?: OverageBehavior
  thresholds: number[]
}

/**
 * A metered value as written back, without the terms that act on nothing (the server refuses
 * them): no limit means Block with no grace and no thresholds; a sliding window takes none.
 */
export function meteredValueToWrite(
  m: StoredMeteredTerms,
  enabled: boolean
): MessageInitShape<typeof EntitlementValueSchema> {
  const hasLimit = !!m.limit
  const sliding = m.resetPeriod?.Inner?.case === 'slidingWindow'
  return {
    value: {
      case: 'meteredValue' as const,
      value: {
        limit: m.limit,
        resetPeriod: m.resetPeriod,
        enabled,
        groupBy: m.groupBy,
        overageBehavior: hasLimit
          ? m.overageBehavior
          : { Inner: { case: 'block' as const, value: {} } },
        thresholds: hasLimit && !sliding ? m.thresholds : [],
      },
    },
  }
}

/**
 * Convert a resolved or effective entitlement value into the `EntitlementValue.value` oneof
 * shape used by the create/update-entitlement mutations, usage fields dropped. `enabled`
 * defaults to the configured switch, never an auto-disabled one.
 */
export function entitlementValueToSpec(
  value: ResolvedEntitlement['value'] | EffectiveEntitlement['value'],
  enabled: boolean = configuredEnabled(value)
): MessageInitShape<typeof EntitlementValueSchema> {
  if (value.case === 'boolean') {
    return {
      value: {
        case: 'booleanValue' as const,
        value: { enabled },
      },
    }
  }
  if (value.case === 'metered') {
    return meteredValueToWrite(value.value, enabled)
  }
  if (value.case === 'config' && value.value.value) {
    return {
      value: {
        case: 'configValue' as const,
        value: value.value.value,
      },
    }
  }
  return { value: { case: undefined } }
}
