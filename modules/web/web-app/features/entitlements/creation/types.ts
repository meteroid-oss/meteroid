import {
  CalendarUnit,
  EntitlementSpec,
  EntitlementValue,
  OverageBehavior,
  ResolvedEntitlement,
} from '@/rpc/api/entitlements/v1/models_pb'

export const RESET_PERIOD_TYPES = [
  'billingCycle',
  'calendar',
  'fixedWindow',
  'slidingWindow',
  'never',
] as const
export type ResetPeriodType = (typeof RESET_PERIOD_TYPES)[number]

export const OVERAGE_BEHAVIOR_TYPES = ['allow', 'block'] as const
export type OverageBehaviorType = (typeof OVERAGE_BEHAVIOR_TYPES)[number]

type EntitlementValueFields = {
  boolEnabled?: boolean
  limit?: string
  resetPeriodType: ResetPeriodType
  resetUnit?: CalendarUnit
  resetInterval?: number
  overageBehaviorType: OverageBehaviorType
  gracePeriodPct?: number
  warningThresholdPct?: number
  meteredEnabled?: boolean
}

export type PendingEntitlementSpec = {
  // exactly one of featureId (existing) or featureName (new) must be set
  featureId?: string
  featureName?: string
  featureDisplayName: string // shown in the pending list; equals featureName for new features
  featureType: 'boolean' | 'metered'
  metricId?: string // only set when featureName is set and featureType === 'metered'
  // product context for grouping in the UI; populated from the selected feature's product
  productId?: string
  productName?: string
} & EntitlementValueFields

/**
 * Stable identity key for a pending spec. Existing features key off `featureId`; new features
 * key off `featureName`. Returns `null` when neither is set — callers must skip such specs
 * (they cannot be persisted and would collide in any keyed collection).
 */
export function pendingSpecKey(spec: PendingEntitlementSpec): string | null {
  return spec.featureId ?? spec.featureName ?? null
}

// Converts a resolved spec (featureId guaranteed) into the proto EntitlementSpec.
// Mode resolution (Override vs additive Grant) happens server-side now.
export function pendingSpecToEntitlementSpec(
  spec: PendingEntitlementSpec & { featureId: string }
): EntitlementSpec {
  const isBoolean = spec.featureType === 'boolean'

  const resetPeriod = (() => {
    switch (spec.resetPeriodType) {
      case 'billingCycle':
        return { Inner: { case: 'billingCycle' as const, value: {} } }
      case 'never':
        return { Inner: { case: 'never' as const, value: {} } }
      case 'calendar':
        return {
          Inner: { case: 'calendar' as const, value: { unit: spec.resetUnit!, interval: spec.resetInterval! } },
        }
      case 'fixedWindow':
        return {
          Inner: { case: 'fixedWindow' as const, value: { unit: spec.resetUnit!, interval: spec.resetInterval! } },
        }
      case 'slidingWindow':
        return {
          Inner: { case: 'slidingWindow' as const, value: { unit: spec.resetUnit!, interval: spec.resetInterval! } },
        }
    }
  })()

  const buildOverageBehavior = (): OverageBehavior | undefined => {
    if (spec.overageBehaviorType === 'allow') {
      return new OverageBehavior({ Inner: { case: 'allow', value: {} } })
    }
    if (spec.overageBehaviorType === 'block') {
      return new OverageBehavior({
        Inner: { case: 'block', value: { gracePeriodPct: spec.gracePeriodPct } },
      })
    }
    return undefined
  }

  const valueFields = isBoolean
    ? {
        value: {
          case: 'booleanValue' as const,
          value: { enabled: spec.boolEnabled ?? true },
        },
      }
    : {
        value: {
          case: 'meteredValue' as const,
          value: {
            limit: spec.limit || undefined,
            resetPeriod,
            overageBehavior: buildOverageBehavior(),
            warningThresholdPct: spec.warningThresholdPct,
            enabled: spec.meteredEnabled ?? true,
          },
        },
      }

  return new EntitlementSpec({
    featureId: spec.featureId,
    value: new EntitlementValue(valueFields),
  })
}

/**
 * Build a PendingEntitlementSpec from a ResolvedEntitlement (used when pinning or
 * toggling disable on an inherited row in the PendingEntitlementsPanel).
 *
 * The caller may override specific value fields (e.g. flipping `enabled`) by
 * passing them in the optional `overrides` argument.
 */
export function resolvedToPendingSpec(
  r: ResolvedEntitlement,
  overrides?: Partial<EntitlementValueFields>
): PendingEntitlementSpec {
  const featureId = r.feature?.id ?? ''
  const featureDisplayName = r.feature?.name ?? featureId
  const productId = r.feature?.product?.id
  const productName = r.feature?.product?.name

  if (r.value.case === 'metered') {
    const m = r.value.value
    const rp = m.resetPeriod?.Inner

    // Derive the resetPeriodType string from the proto oneof
    let resetPeriodType: EntitlementValueFields['resetPeriodType'] = 'never'
    let resetUnit: CalendarUnit | undefined
    let resetInterval: number | undefined
    if (rp?.case === 'billingCycle') {
      resetPeriodType = 'billingCycle'
    } else if (rp?.case === 'calendar') {
      resetPeriodType = 'calendar'
      resetUnit = rp.value.unit
      resetInterval = rp.value.interval
    } else if (rp?.case === 'fixedWindow') {
      resetPeriodType = 'fixedWindow'
      resetUnit = rp.value.unit
      resetInterval = rp.value.interval
    } else if (rp?.case === 'slidingWindow') {
      resetPeriodType = 'slidingWindow'
      resetUnit = rp.value.unit
      resetInterval = rp.value.interval
    }

    // Derive overage behavior. Inherited entitlements without an explicit policy
    // default to `block` — the safer choice (matches the dialog's default value).
    const ob = m.overageBehavior?.Inner
    let overageBehaviorType: EntitlementValueFields['overageBehaviorType'] = 'block'
    let gracePeriodPct: number | undefined
    if (ob?.case === 'allow') {
      overageBehaviorType = 'allow'
    } else if (ob?.case === 'block') {
      overageBehaviorType = 'block'
      gracePeriodPct = ob.value.gracePeriodPct
    }

    return {
      featureId,
      featureDisplayName,
      featureType: 'metered',
      productId,
      productName,
      resetPeriodType,
      resetUnit,
      resetInterval,
      overageBehaviorType,
      gracePeriodPct,
      limit: m.limit,
      warningThresholdPct: m.warningThresholdPct,
      meteredEnabled: m.enabled,
      ...overrides,
    }
  }

  // boolean (or undefined) case
  const enabled = r.value.case === 'boolean' ? r.value.value.enabled : true
  return {
    featureId,
    featureDisplayName,
    featureType: 'boolean',
    productId,
    productName,
    resetPeriodType: 'never',
    // overageBehaviorType is metered-only; set the default to satisfy the type — it is
    // never read for boolean features.
    overageBehaviorType: 'block',
    boolEnabled: enabled,
    ...overrides,
  }
}
