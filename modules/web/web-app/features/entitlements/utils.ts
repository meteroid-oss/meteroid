import Decimal from 'decimal.js'

import {
  CalendarUnit,
  ConfigValue,
  EffectiveEntitlement,
  OverageBehavior,
  ResetPeriod,
  ResolvedEntitlement,
  ResolvedOrigin,
} from '@/rpc/api/entitlements/v1/models_pb'

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

/** What a limit does once reached, worded as the feature's policy is, e.g. "Deny at limit + 10% grace". */
export function limitBehaviorLabel(t: LimitTerms): string | undefined {
  if (!t.limit) return undefined
  if (t.overage === 'allow') return 'Overage allowed'
  return t.gracePct ? `Deny at limit + ${t.gracePct}% grace` : 'Deny at limit'
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
