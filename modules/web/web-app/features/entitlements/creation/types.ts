import { create, type MessageInitShape } from '@bufbuild/protobuf';
import { z } from 'zod';

import { formatThresholds, parseThresholds } from '@/features/thresholds/thresholds'
import {
  CalendarUnit,
  ConfigValueSchema,
  ConfigValueType,
  EntitlementSpecSchema,
  EntitlementValueSchema,
  OverageBehaviorSchema,
  ResetPeriodSchema,
  ResolvedEntitlement,
} from '@/rpc/api/entitlements/v1/models_pb';

import type { EntitlementSpec, OverageBehavior } from '@/rpc/api/entitlements/v1/models_pb';

export const CONFIG_KINDS = ['number', 'boolean', 'text', 'map', 'json', 'select'] as const
export type ConfigKind = (typeof CONFIG_KINDS)[number]

/** Map a UI config kind to the proto `ConfigValueType` enum. */
export function configKindToProto(kind: ConfigKind): ConfigValueType {
  switch (kind) {
    case 'number':
      return ConfigValueType.NUMBER
    case 'boolean':
      return ConfigValueType.BOOLEAN
    case 'map':
      return ConfigValueType.MAP
    case 'json':
      return ConfigValueType.JSON
    case 'select':
      return ConfigValueType.SELECT
    case 'text':
    default:
      return ConfigValueType.TEXT
  }
}

/** Build the proto `ConfigFeature` payload (value_type + options) for a create-feature request. */
export function configFeatureTypeValue(kind: ConfigKind, options: string[]) {
  return {
    valueType: configKindToProto(kind),
    options: kind === 'select' ? options : [],
  }
}

// A `map` config feature stores several typed key/value pairs in one entitlement,
// persisted as the `json` ConfigValue (a flat object). Lets you define e.g. a limit per property.
export const CONFIG_ENTRY_TYPES = ['number', 'text', 'boolean'] as const
export type ConfigEntryType = (typeof CONFIG_ENTRY_TYPES)[number]
export type ConfigEntry = { key: string; type: ConfigEntryType; value: string }

/** Parse a `json` ConfigValue into map entries, or `null` if it isn't a flat object of primitives. */
export function parseConfigMap(jsonStr: string): ConfigEntry[] | null {
  let parsed: unknown
  try {
    parsed = JSON.parse(jsonStr)
  } catch {
    return null
  }
  if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) return null
  const entries: ConfigEntry[] = []
  for (const [key, val] of Object.entries(parsed)) {
    if (typeof val === 'number') entries.push({ key, type: 'number', value: String(val) })
    else if (typeof val === 'boolean') entries.push({ key, type: 'boolean', value: val ? 'true' : 'false' })
    else if (typeof val === 'string') entries.push({ key, type: 'text', value: val })
    else return null // nested object/array ⇒ keep as raw JSON, not a flat map
  }
  return entries
}

/** Validate map entries; returns the first user-facing error message, or `null` if valid. */
export function configMapIssues(entries: ConfigEntry[] | undefined): string | null {
  const list = entries ?? []
  if (list.length === 0) return 'Add at least one entry'
  const seen = new Set<string>()
  for (const e of list) {
    const key = e.key.trim()
    if (!key) return 'Every entry needs a key'
    if (seen.has(key)) return `Duplicate key: ${key}`
    seen.add(key)
    if (e.type === 'number' && (e.value.trim() === '' || !Number.isFinite(Number(e.value))))
      return `Entry "${key}" needs a numeric value`
  }
  return null
}

export const GROUP_BY_MAX_LENGTH = 128

// Mirrors the server's group property rules (plain text, no comma, at most 128 bytes).
export const groupBySchema = z
  .string()
  .max(GROUP_BY_MAX_LENGTH, `At most ${GROUP_BY_MAX_LENGTH} characters`)
  .refine(v => v === '' || v.trim() === v, 'No leading or trailing spaces')
  .refine(v => !v.includes(','), 'A single property, without commas')
  .optional()

export const OVERAGE_TYPES = ['block', 'allow'] as const
export type OverageType = (typeof OVERAGE_TYPES)[number]

export const MAX_GRACE_PCT = 1000

const isPercent = (v: string | undefined, max: number) =>
  !v?.trim() || (/^\d+$/.test(v.trim()) && +v.trim() >= 1 && +v.trim() <= max)

// Checked by `checkMeteredPolicy`, only where the field is shown.
export const meteredPolicySchemaShape = {
  overageType: z.enum(OVERAGE_TYPES).optional(),
  gracePct: z.string().optional(),
  thresholds: z.string().optional(),
}

/** Validates the grace and alert thresholds of a metered value, only where they are shown. */
export const checkMeteredPolicy = (
  d: {
    limit?: string
    resetPeriodType?: ResetPeriodType
    overageType?: OverageType
    gracePct?: string
    thresholds?: string
  },
  ctx: z.RefinementCtx
) => {
  if (
    limitSettingsApply(d.limit) &&
    d.overageType !== 'allow' &&
    !isPercent(d.gracePct, MAX_GRACE_PCT)
  )
    ctx.addIssue({
      code: 'custom',
      path: ['gracePct'],
      message: `Enter a whole percentage between 1 and ${MAX_GRACE_PCT}`,
    })
  if (warningApplies(d)) {
    try {
      parseThresholds(d.thresholds ?? '')
    } catch (error) {
      ctx.addIssue({ code: 'custom', path: ['thresholds'], message: (error as Error).message })
    }
  }
}

export const RESET_PERIOD_TYPES = [
  'billingCycle',
  'calendar',
  'fixedWindow',
  'slidingWindow',
  'never',
] as const
export type ResetPeriodType = (typeof RESET_PERIOD_TYPES)[number]

type EntitlementValueFields = {
  boolEnabled?: boolean
  limit?: string
  resetPeriodType: ResetPeriodType
  resetUnit?: CalendarUnit
  resetInterval?: number
  meteredEnabled?: boolean
  // Metric property the limit applies to separately (e.g. per project); empty means one limit.
  groupBy?: string
  // At the limit: block (optionally with a grace, in % of the limit) or allow. Empty = no grace.
  overageType?: OverageType
  gracePct?: string
  // Percents of the limit at which an alert is sent, comma-separated; empty sends none before 100%.
  thresholds?: string
  // Config (static value) fields. `configKind` selects which typed field applies.
  configKind?: ConfigKind
  configNumber?: string
  configBoolean?: boolean
  configText?: string
  configJson?: string
  // Typed key/value pairs for a `map` config value (serialized to the `json` ConfigValue).
  configEntries?: ConfigEntry[]
  // Allowed values for a `select` config feature (stored on the feature, not the value).
  configOptions?: string[]
}

export type PendingEntitlementSpec = {
  // exactly one of featureId (existing) or featureName (new) must be set
  featureId?: string
  featureName?: string
  featureCode?: string // stable code for a new feature; only set alongside featureName
  featureDisplayName: string // shown in the pending list; equals featureName for new features
  featureType: 'boolean' | 'metered' | 'config'
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

// Build the proto ConfigValue oneof from typed config fields. Shared by the creation
// wizard and the entity-entitlement dialog.
export function buildConfigValue(fields: {
  configKind?: ConfigKind
  configNumber?: string
  configBoolean?: boolean
  configText?: string
  configJson?: string
  configEntries?: ConfigEntry[]
}): MessageInitShape<typeof ConfigValueSchema> {
  switch (fields.configKind) {
    case 'number':
      return { value: { case: 'number' as const, value: (fields.configNumber ?? '').trim() || '0' } }
    case 'boolean':
      return { value: { case: 'boolean' as const, value: fields.configBoolean ?? false } }
    case 'map': {
      // Serialize typed key/value rows into a flat JSON object, stored as the `json` ConfigValue.
      const obj: Record<string, string | number | boolean> = {}
      for (const e of fields.configEntries ?? []) {
        const key = e.key.trim()
        if (!key) continue
        obj[key] =
          e.type === 'number' ? Number(e.value)
          : e.type === 'boolean' ? e.value === 'true'
          : e.value
      }
      return { value: { case: 'json' as const, value: JSON.stringify(obj) } }
    }
    case 'json':
      return { value: { case: 'json' as const, value: (fields.configJson ?? '').trim() || 'null' } }
    // A select stores the chosen option as a plain text value; the allowed options live on
    // the feature (config_options), not on each entitlement value.
    case 'select':
    case 'text':
    default:
      return { value: { case: 'text' as const, value: fields.configText ?? '' } }
  }
}

export type MeteredPolicyValues = Pick<
  EntitlementValueFields,
  'overageType' | 'gracePct' | 'thresholds'
>

const parsePct = (v: string | undefined) => (v?.trim() ? Number(v.trim()) : undefined)

/** Settings that only act on a limit: the overage terms and the alert thresholds. */
export const limitSettingsApply = (limit: string | undefined): boolean => !!limit?.trim()

/** A sliding window sends no alerts, so it takes no alert thresholds. */
export const warningApplies = (fields: {
  limit?: string
  resetPeriodType?: ResetPeriodType
}): boolean => limitSettingsApply(fields.limit) && fields.resetPeriodType !== 'slidingWindow'

/** The overage terms and thresholds, dropped where they would do nothing (the server refuses them). */
export function buildMeteredPolicy(
  fields: MeteredPolicyValues & { limit?: string; resetPeriodType?: ResetPeriodType }
): {
  overageBehavior: MessageInitShape<typeof OverageBehaviorSchema>
  thresholds: number[]
} {
  if (!limitSettingsApply(fields.limit)) {
    return {
      overageBehavior: { Inner: { case: 'block' as const, value: {} } },
      thresholds: [],
    }
  }
  return {
    overageBehavior:
      fields.overageType === 'allow'
        ? { Inner: { case: 'allow' as const, value: {} } }
        : {
            Inner: {
              case: 'block' as const,
              value: { gracePeriodPct: parsePct(fields.gracePct) },
            },
          },
    thresholds: warningApplies(fields) ? parseThresholds(fields.thresholds ?? '') : [],
  }
}

export function meteredPolicyToFields(m: {
  overageBehavior?: OverageBehavior
  thresholds: number[]
}): Required<MeteredPolicyValues> {
  const inner = m.overageBehavior?.Inner
  return {
    overageType: inner?.case === 'allow' ? 'allow' : 'block',
    gracePct:
      inner?.case === 'block' && inner.value.gracePeriodPct !== undefined
        ? String(inner.value.gracePeriodPct)
        : '',
    thresholds: formatThresholds(m.thresholds),
  }
}

export type ResetPeriodValues = Pick<
  EntitlementValueFields,
  'resetPeriodType' | 'resetUnit' | 'resetInterval'
>

export function buildResetPeriod(fields: ResetPeriodValues): MessageInitShape<typeof ResetPeriodSchema> {
  switch (fields.resetPeriodType) {
    case 'billingCycle':
      return { Inner: { case: 'billingCycle' as const, value: {} } }
    case 'never':
      return { Inner: { case: 'never' as const, value: {} } }
    case 'calendar':
    case 'fixedWindow':
    case 'slidingWindow':
      return {
        Inner: {
          case: fields.resetPeriodType,
          value: { unit: fields.resetUnit!, interval: fields.resetInterval! },
        },
      }
  }
}

// Converts a resolved spec (featureId guaranteed) into the proto EntitlementSpec.
// Mode resolution (Override vs additive Grant) happens server-side now.
export function pendingSpecToEntitlementSpec(
  spec: PendingEntitlementSpec & { featureId: string }
): EntitlementSpec {
  const resetPeriod = buildResetPeriod(spec)

  const valueFields: MessageInitShape<typeof EntitlementValueSchema> =
    spec.featureType === 'boolean'
      ? {
          value: {
            case: 'booleanValue' as const,
            value: { enabled: spec.boolEnabled ?? true },
          },
        }
      : spec.featureType === 'config'
        ? {
            value: {
              case: 'configValue' as const,
              value: buildConfigValue(spec),
            },
          }
        : {
            value: {
              case: 'meteredValue' as const,
              value: {
                limit: spec.limit || undefined,
                resetPeriod,
                enabled: spec.meteredEnabled ?? true,
                groupBy: spec.groupBy || undefined,
                ...buildMeteredPolicy(spec),
              },
            },
          }

  return create(EntitlementSpecSchema, {
    featureId: spec.featureId,
    value: create(EntitlementValueSchema, valueFields),
  });
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

    return {
      featureId,
      featureDisplayName,
      featureType: 'metered',
      productId,
      productName,
      resetPeriodType,
      resetUnit,
      resetInterval,
      limit: m.limit,
      meteredEnabled: m.enabled,
      groupBy: m.groupBy,
      ...meteredPolicyToFields(m),
      ...overrides,
    }
  }

  if (r.value.case === 'config') {
    const cv = r.value.value.value?.value
    // `ResolvedEntitlement.feature` is a FeatureRef without the value type, so derive the editor
    // kind from the stored value (a flat JSON object rounds-trips to the map editor).
    let configKind: ConfigKind = 'text'
    let configNumber: string | undefined
    let configBoolean: boolean | undefined
    let configText: string | undefined
    let configJson: string | undefined
    let configEntries: ConfigEntry[] | undefined
    if (cv?.case === 'number') {
      configKind = 'number'
      configNumber = cv.value
    } else if (cv?.case === 'boolean') {
      configKind = 'boolean'
      configBoolean = cv.value
    } else if (cv?.case === 'json') {
      configJson = cv.value
      configEntries = parseConfigMap(cv.value) ?? undefined
      configKind = configEntries ? 'map' : 'json'
    } else if (cv?.case === 'text') {
      configKind = 'text'
      configText = cv.value
    }
    return {
      featureId,
      featureDisplayName,
      featureType: 'config',
      productId,
      productName,
      resetPeriodType: 'never',
      configKind,
      configNumber,
      configBoolean,
      configText,
      configJson,
      configEntries,
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
    boolEnabled: enabled,
    ...overrides,
  }
}
