import {
  CalendarUnit,
  EffectiveEntitlement,
  EntitlementValue,
  FeatureType,
  OverageBehavior,
  ResetPeriod,
  ResolvedEntitlement,
  ResolvedOrigin,
} from '@/rpc/api/entitlements/v1/models_pb'

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

// Local domain types — use these in UI, map from proto at the boundary

export type BooleanFeatureKind = { type: 'boolean' }
export type MeteredFeatureKind = { type: 'metered'; metricId: string }
export type FeatureKind = BooleanFeatureKind | MeteredFeatureKind

export function featureKindFromProto(ft: FeatureType | undefined): FeatureKind {
  if (ft?.Inner?.case === 'metered') {
    return { type: 'metered', metricId: ft.Inner.value.metricId }
  }
  return { type: 'boolean' }
}

export function featureTypeLabel(kind: FeatureKind): string {
  return kind.type === 'metered' ? 'Metered' : 'Boolean'
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

export function overageBehaviorLabel(ob: OverageBehavior): string {
  const inner = ob.Inner
  if (!inner || inner.case === undefined) return '—'
  if (inner.case === 'allow') return 'Allow overage'
  if (inner.case === 'block') {
    const grace = inner.value.gracePeriodPct
    return grace != null ? `Block at cap (+${grace}% grace)` : 'Block at cap'
  }
  return '—'
}

export function entitlementValueLabel(value: EntitlementValue['value'] | undefined): string {
  if (value?.case === 'booleanValue') {
    return value.value.enabled ? '✓ Enabled' : '✗ Disabled'
  }
  if (value?.case === 'meteredValue') {
    const m = value.value
    const limitPart = m.limit ? m.limit : '∞ Unlimited'
    const periodPart = m.resetPeriod ? ` / ${resetPeriodLabel(m.resetPeriod)}` : ''
    const warnPart = m.warningThresholdPct != null ? ` · warn ${m.warningThresholdPct}%` : ''
    return `${limitPart}${periodPart}${warnPart}`
  }
  return '—'
}

// ── Resolved-entitlements helpers ─────────────────────────────────────────────

/**
 * True when the entitlement's kill-switch is engaged (boolean disabled or metered
 * `enabled = false`). Used to dim rows and gate "Enable/Disable" menu items.
 */
export function isEntitlementDisabled(
  value: ResolvedEntitlement['value'] | EffectiveEntitlement['value']
): boolean {
  if (value.case === 'boolean') return !value.value.enabled
  if (value.case === 'metered') return !value.value.enabled
  return false
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
    default:
      return ''
  }
}

/**
 * Compact display string for a resolved entitlement value — used in table rows.
 */
export function formatResolvedValue(value: ResolvedEntitlement['value']): string {
  if (value.case === 'boolean') {
    return value.value.enabled ? 'Enabled' : 'Disabled'
  }
  if (value.case === 'metered') {
    const m = value.value
    const limit = m.limit ?? '∞'
    const warn = m.warningThresholdPct != null ? ` (warn @ ${m.warningThresholdPct}%)` : ''
    return `${limit}${warn}`
  }
  return '—'
}

/**
 * Convert a resolved or effective entitlement value into the `EntitlementValue.value`
 * oneof shape used by the create/update-entitlement mutations. The spec fields
 * (limit, reset_period, …) are 1-to-1 compatible across both inputs; usage fields on
 * `EffectiveEntitlement` are dropped.
 */
export function entitlementValueToSpec(
  value: ResolvedEntitlement['value'] | EffectiveEntitlement['value']
): ConstructorParameters<typeof EntitlementValue>[0] {
  if (value.case === 'boolean') {
    return {
      value: {
        case: 'booleanValue' as const,
        value: { enabled: value.value.enabled },
      },
    }
  }
  if (value.case === 'metered') {
    const m = value.value
    return {
      value: {
        case: 'meteredValue' as const,
        value: {
          limit: m.limit,
          resetPeriod: m.resetPeriod,
          overageBehavior: m.overageBehavior,
          warningThresholdPct: m.warningThresholdPct,
          enabled: m.enabled,
        },
      },
    }
  }
  return { value: { case: undefined } }
}
