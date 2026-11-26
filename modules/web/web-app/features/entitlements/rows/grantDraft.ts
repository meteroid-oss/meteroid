import { type JsonValue, type MessageInitShape, clone, create, fromJson, toJson } from '@bufbuild/protobuf'

import { type Source, defaultEffect, isOrdered } from '@/features/entitlements/model'
import {
  type ConfigValueSchema,
  type Entitlement,
  EntitlementEffect,
  EntitlementSchema,
  type EntitlementSpec,
  EntitlementSpecSchema,
  type EntitlementValue,
  EntitlementValueSchema,
  type Feature,
  FeatureKind,
  FeatureSchema,
  FeatureValueType,
  type ResolvedEntitlement,
} from '@/rpc/api/entitlements/v1/models_pb'

/**
 * An entitlement collected before its owner exists, submitted with the owner's create request.
 * `spec.effect` stays UNSPECIFIED while the user keeps the source's default, so the server applies it.
 */
export type GrantDraft = { feature: Feature; spec: EntitlementSpec }

type ValueInit = MessageInitShape<typeof EntitlementValueSchema>

export const draftsToSpecs = (drafts: GrantDraft[]): EntitlementSpec[] => drafts.map(d => d.spec)

export const draftEffect = (draft: GrantDraft, source: Source, multiInstance: boolean) =>
  draft.spec.effect === EntitlementEffect.UNSPECIFIED
    ? defaultEffect(source, isOrdered(draft.feature), multiInstance)
    : draft.spec.effect

/** The draft as a row, for the table and the drawer. */
export const draftRow = (draft: GrantDraft, effect: EntitlementEffect): Entitlement => {
  const s = draft.spec
  return create(EntitlementSchema, {
    featureId: s.featureId,
    value: s.value,
    effect,
    level: s.level,
    levelTypeId: s.levelTypeId,
    condition: s.condition,
    validFrom: s.validFrom,
    validUntil: s.validUntil,
    reason: s.reason,
    policyOverride: s.policyOverride,
  })
}

export const newDraft = (feature: Feature, value: ValueInit): GrantDraft => ({
  feature,
  spec: create(EntitlementSpecSchema, { featureId: feature.id, value }),
})

/** Whether the value is switched on; config values have no switch. */
export const isEnabled = (v: EntitlementValue | undefined) => {
  const inner = v?.value
  if (inner?.case === 'booleanValue' || inner?.case === 'meteredValue') return inner.value.enabled
  return true
}

export const hasSwitch = (v: EntitlementValue | undefined) =>
  v?.value.case === 'booleanValue' || v?.value.case === 'meteredValue'

export const withEnabled = (draft: GrantDraft, enabled: boolean): GrantDraft => {
  const inner = draft.spec.value?.value
  const value: ValueInit | undefined =
    inner?.case === 'booleanValue'
      ? { value: { case: 'booleanValue', value: { enabled } } }
      : inner?.case === 'meteredValue'
        ? { value: { case: 'meteredValue', value: { limit: inner.value.limit, enabled } } }
        : undefined
  if (!value) return draft
  const spec = clone(EntitlementSpecSchema, draft.spec)
  spec.value = create(EntitlementValueSchema, value)
  return { ...draft, spec }
}

// ── Inherited rows ────────────────────────────────────────────────────────────

/** A feature known only by its reference, its kind read from a value. */
const featureOf = (
  ref: { id: string; name: string; code?: string; product?: { id: string; name: string } },
  value: ValueInit
): Feature => {
  const inner = value.value
  const config = inner?.case === 'configValue' ? inner.value.value?.case : undefined
  return create(FeatureSchema, {
    id: ref.id,
    name: ref.name,
    code: ref.code ?? '',
    product: ref.product,
    kind:
      inner?.case === 'booleanValue'
        ? FeatureKind.BOOLEAN
        : inner?.case === 'meteredValue'
          ? FeatureKind.LIMIT
          : FeatureKind.VALUE,
    value:
      inner?.case === 'configValue'
        ? {
            type:
              config === 'number'
                ? FeatureValueType.NUMBER
                : config === 'json'
                  ? FeatureValueType.JSON
                  : FeatureValueType.TEXT,
          }
        : undefined,
  })
}

/** The inherited value, as the old creation flows pinned it. */
export function inheritedValue(r: ResolvedEntitlement): ValueInit {
  switch (r.value.case) {
    case 'metered':
      return {
        value: {
          case: 'meteredValue',
          value: { limit: r.value.value.limit || undefined, enabled: r.value.value.enabled },
        },
      }
    case 'config': {
      const cv = r.value.value.value?.value
      const entries = cv?.case === 'json' ? parseConfigMap(cv.value) : null
      const config: LegacyConfig =
        cv?.case === 'number'
          ? { configKind: 'number', configNumber: cv.value }
          : cv?.case === 'boolean'
            ? { configKind: 'boolean', configBoolean: cv.value }
            : cv?.case === 'json'
              ? entries
                ? { configKind: 'map', configEntries: entries }
                : { configKind: 'json', configJson: cv.value }
              : { configKind: 'text', configText: cv?.case === 'text' ? cv.value : undefined }
      return { value: { case: 'configValue', value: legacyConfigValue(config) } }
    }
    case 'boolean':
      return { value: { case: 'booleanValue', value: { enabled: r.value.value.enabled } } }
    default:
      return { value: { case: 'booleanValue', value: { enabled: true } } }
  }
}

/** A draft holding the inherited value; `feature` when loaded, else one read from the reference. */
export const inheritedDraft = (r: ResolvedEntitlement, feature?: Feature): GrantDraft => {
  const value = inheritedValue(r)
  const ref = r.feature ?? { id: '', name: '', code: '' }
  return newDraft(feature ?? featureOf(ref, value), value)
}

// ── Persistence (quote drafts and templates) ──────────────────────────────────

export const encodeDrafts = (drafts: GrantDraft[]): JsonValue[] =>
  drafts.map(d => ({ feature: toJson(FeatureSchema, d.feature), spec: toJson(EntitlementSpecSchema, d.spec) }))

/** Stored drafts, including those the old creation dialog wrote. */
export function decodeDrafts(raw: unknown): GrantDraft[] {
  if (!Array.isArray(raw)) return []
  return raw.flatMap((item): GrantDraft[] => {
    if (!item || typeof item !== 'object') return []
    try {
      if ('spec' in item && 'feature' in item) {
        const options = { ignoreUnknownFields: true }
        return [
          {
            feature: fromJson(FeatureSchema, item.feature as JsonValue, options),
            spec: fromJson(EntitlementSpecSchema, item.spec as JsonValue, options),
          },
        ]
      }
      return legacyDraft(item as LegacySpec)
    } catch {
      return []
    }
  })
}

type LegacyConfig = {
  configKind?: string
  configNumber?: string
  configBoolean?: boolean
  configText?: string
  configJson?: string
  configEntries?: { key: string; type: string; value: string }[]
}

type LegacySpec = LegacyConfig & {
  featureId?: string
  featureDisplayName?: string
  featureType?: 'boolean' | 'metered' | 'config'
  productId?: string
  productName?: string
  boolEnabled?: boolean
  limit?: string
  meteredEnabled?: boolean
}

// A spec naming a feature to create (no `featureId`) can't be kept: features are created first now.
function legacyDraft(s: LegacySpec): GrantDraft[] {
  if (!s.featureId || !s.featureType) return []
  const value: ValueInit =
    s.featureType === 'boolean'
      ? { value: { case: 'booleanValue', value: { enabled: s.boolEnabled ?? true } } }
      : s.featureType === 'config'
        ? { value: { case: 'configValue', value: legacyConfigValue(s) } }
        : {
            value: {
              case: 'meteredValue',
              value: { limit: s.limit || undefined, enabled: s.meteredEnabled ?? true },
            },
          }
  const product = s.productId ? { id: s.productId, name: s.productName ?? '' } : undefined
  return [newDraft(featureOf({ id: s.featureId, name: s.featureDisplayName ?? s.featureId, product }, value), value)]
}

function legacyConfigValue(c: LegacyConfig): MessageInitShape<typeof ConfigValueSchema> {
  switch (c.configKind) {
    case 'number':
      return { value: { case: 'number', value: (c.configNumber ?? '').trim() || '0' } }
    case 'boolean':
      return { value: { case: 'boolean', value: c.configBoolean ?? false } }
    case 'map': {
      const obj: Record<string, string | number | boolean> = {}
      for (const e of c.configEntries ?? []) {
        const key = e.key.trim()
        if (!key) continue
        obj[key] = e.type === 'number' ? Number(e.value) : e.type === 'boolean' ? e.value === 'true' : e.value
      }
      return { value: { case: 'json', value: JSON.stringify(obj) } }
    }
    case 'json':
      return { value: { case: 'json', value: (c.configJson ?? '').trim() || 'null' } }
    default:
      return { value: { case: 'text', value: c.configText ?? '' } }
  }
}

/** A flat JSON object of primitives as typed entries, or null. */
function parseConfigMap(json: string): LegacyConfig['configEntries'] | null {
  let parsed: unknown
  try {
    parsed = JSON.parse(json)
  } catch {
    return null
  }
  if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) return null
  const entries: NonNullable<LegacyConfig['configEntries']> = []
  for (const [key, val] of Object.entries(parsed)) {
    if (typeof val === 'number') entries.push({ key, type: 'number', value: String(val) })
    else if (typeof val === 'boolean') entries.push({ key, type: 'boolean', value: val ? 'true' : 'false' })
    else if (typeof val === 'string') entries.push({ key, type: 'text', value: val })
    else return null
  }
  return entries
}
