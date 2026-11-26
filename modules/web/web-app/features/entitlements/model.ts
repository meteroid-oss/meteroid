import { create } from '@bufbuild/protobuf'
import Decimal from 'decimal.js'
import { z } from 'zod'

import { type TypeList, attributeKeyLabel, lower } from '@/features/resources/hooks'
import { formatThresholds, parseThresholds } from '@/features/thresholds/thresholds'
import {
  CalendarUnit,
  CheckReason,
  type Condition,
  DiscardReason,
  type Entitlement,
  EntitlementEffect,
  type EntitlementEntity,
  EntitlementLevel,
  type EntitlementValue,
  EntitlementValueSchema,
  type Feature,
  FeatureKind,
  type FeaturePolicy,
  type FeaturePolicySchema,
  FeatureValueType,
  LimitCountedBy,
  PolicyAtLimit,
  PolicyResets,
  type ResolvedEntitlement,
  Tier,
} from '@/rpc/api/entitlements/v1/models_pb'

import { configValueLabel, formatDecimalGrouped } from './utils'

import type { ResourceType } from '@/rpc/api/resources/v1/models_pb'
import type { MessageInitShape } from '@bufbuild/protobuf'

type Types = Map<string, ResourceType>

// ── Features ──────────────────────────────────────────────────────────────────

export const isLimit = (f: Feature | undefined) => f?.kind === FeatureKind.LIMIT
export const isAppCounted = (f: Feature | undefined) =>
  isLimit(f) && f?.limit?.countedBy === LimitCountedBy.APP

/** Text, json and unordered select values can't be compared: they only take Set to. */
export const isOrdered = (f: Feature | undefined) =>
  f?.kind !== FeatureKind.VALUE ||
  f.value?.type === FeatureValueType.NUMBER ||
  (f.value?.type === FeatureValueType.SELECT && f.value.ordered)

const VALUE_TYPE_LABEL: Record<FeatureValueType, string> = {
  [FeatureValueType.UNSPECIFIED]: 'Value',
  [FeatureValueType.NUMBER]: 'Number',
  [FeatureValueType.TEXT]: 'Text',
  [FeatureValueType.SELECT]: 'Select',
  [FeatureValueType.JSON]: 'JSON',
}

export const kindLabel = (f: Feature) =>
  f.kind === FeatureKind.LIMIT
    ? 'Limit'
    : f.kind === FeatureKind.VALUE
      ? VALUE_TYPE_LABEL[f.value?.type ?? FeatureValueType.UNSPECIFIED]
      : 'On/off'

/** "Per site", or "Customer" for a feature applying to the customer as a whole. */
export const appliesPerLabel = (f: Feature, types: Types) => {
  const type = f.levelResourceTypeId ? types.get(f.levelResourceTypeId) : undefined
  return type ? `Per ${lower(type.name)}` : 'Customer'
}

// ── Sources and effects ───────────────────────────────────────────────────────

export type Owner = Exclude<EntitlementEntity['EntityId'], { case: undefined }>
export type Source = 'default' | 'plan' | 'subscription' | 'addOn' | 'customer' | 'resource'

export const sourceOf = (owner: Owner, resourceId?: string): Source => {
  switch (owner.case) {
    case 'featureId':
      return 'default'
    case 'planId':
    case 'planVersionId':
      return 'plan'
    case 'subscriptionId':
    case 'quoteId':
      return 'subscription'
    case 'addOnId':
      return 'addOn'
    case 'customerId':
      return resourceId ? 'resource' : 'customer'
  }
}

export const EFFECT: Record<EntitlementEffect, { label: string; hint: string }> = {
  [EntitlementEffect.UNSPECIFIED]: { label: 'Set to', hint: 'Exactly this, whatever the plan and add-ons give' },
  [EntitlementEffect.SET]: { label: 'Set to', hint: 'Exactly this, whatever the plan and add-ons give' },
  [EntitlementEffect.ADD]: { label: 'Add', hint: 'Once per add-on bought' },
  [EntitlementEffect.RAISE_TO]: { label: 'Raise to', hint: 'Never less than this' },
  [EntitlementEffect.CAP_AT]: { label: 'Cap at', hint: 'Never more than this' },
}

export function allowedEffects(source: Source, ordered: boolean): EntitlementEffect[] {
  if (!ordered) return [EntitlementEffect.SET]
  if (source === 'addOn') return [EntitlementEffect.ADD, EntitlementEffect.RAISE_TO]
  if (source === 'customer' || source === 'resource')
    return [EntitlementEffect.RAISE_TO, EntitlementEffect.SET, EntitlementEffect.CAP_AT]
  return [EntitlementEffect.SET]
}

export const defaultEffect = (source: Source, ordered: boolean, multiInstance: boolean) =>
  !ordered
    ? EntitlementEffect.SET
    : source === 'addOn' && multiInstance
      ? EntitlementEffect.ADD
      : allowedEffects(source, ordered)[0]

export const effectOf = (e: Pick<Entitlement, 'effect'>) =>
  e.effect === EntitlementEffect.UNSPECIFIED ? EntitlementEffect.SET : e.effect

// ── Levels ────────────────────────────────────────────────────────────────────

/** A row's level as one select value: the feature's own, across all resources, or one type. */
export type LevelKey = 'feature' | 'customer' | `type:${string}`

export const levelKeyOf = (e: Pick<Entitlement, 'level' | 'levelTypeId'>): LevelKey =>
  e.level === EntitlementLevel.CUSTOMER
    ? 'customer'
    : e.level === EntitlementLevel.TYPE && e.levelTypeId
      ? `type:${e.levelTypeId}`
      : 'feature'

export const levelFields = (key: LevelKey) =>
  key === 'feature'
    ? { level: EntitlementLevel.FEATURE }
    : key === 'customer'
      ? { level: EntitlementLevel.CUSTOMER }
      : { level: EntitlementLevel.TYPE, levelTypeId: key.slice(5) }

/** The type a row's level stands for, unset for the customer. Its attributes are the condition's. */
export const levelTypeId = (key: LevelKey, f: Feature | undefined) =>
  key === 'feature' ? f?.levelResourceTypeId : key === 'customer' ? undefined : key.slice(5)

export function levelLabel(key: LevelKey, f: Feature | undefined, types: Types): string {
  const typeId = levelTypeId(key, f)
  const type = typeId ? types.get(typeId) : undefined
  if (type) return `Per ${lower(type.name)}`
  const own = f?.levelResourceTypeId ? types.get(f.levelResourceTypeId) : undefined
  return key === 'customer' && own ? `Across all ${lower(own.pluralName)}` : 'Customer'
}

/** The levels a new row of this feature may take, own level first. */
export function levelOptions(f: Feature, source: Source, soldForTypeId: string | undefined, types: ResourceType[]) {
  const keys: LevelKey[] = ['feature']
  if (!isOrdered(f) || source === 'resource') return keys
  if (f.levelResourceTypeId) {
    // Customer-wide caps belong on customer plans and overrides, never on a plan sold for a type.
    if (!(source === 'plan' && soldForTypeId)) keys.push('customer')
  } else {
    keys.push(...types.map(t => `type:${t.id}` as const))
  }
  return keys
}

// ── Values ────────────────────────────────────────────────────────────────────

export function valueLabel(v: EntitlementValue | undefined, f?: Feature): string {
  const inner = v?.value
  switch (inner?.case) {
    case 'booleanValue':
      return inner.value.enabled ? 'on' : 'off'
    case 'meteredValue':
      if (!inner.value.enabled) return 'off'
      if (!inner.value.limit) return 'unlimited'
      return f?.unit
        ? `${formatDecimalGrouped(inner.value.limit)} ${f.unit}`
        : formatDecimalGrouped(inner.value.limit)
    case 'configValue':
      return configValueLabel(inner.value)
    default:
      return '—'
  }
}

/** A value's place in the feature's order, higher is better; unset when unordered. */
export function valueRank(v: EntitlementValue | undefined, f: Feature | undefined): number | undefined {
  const inner = v?.value
  switch (inner?.case) {
    case 'booleanValue':
      return inner.value.enabled ? 1 : 0
    case 'meteredValue':
      if (!inner.value.enabled) return -1
      return inner.value.limit ? Number(inner.value.limit) : Infinity
    case 'configValue': {
      const c = inner.value.value
      if (c.case === 'number') return Number(c.value) * (f?.value?.higherIsBetter === false ? -1 : 1)
      if (c.case === 'text' && f?.value?.ordered) return f.value.options.indexOf(c.value)
      return undefined
    }
    default:
      return undefined
  }
}

// ── Conditions ────────────────────────────────────────────────────────────────

export type Clauses = Record<string, string[]>

export const clausesOf = (c: Condition | undefined): Clauses =>
  Object.fromEntries(Object.entries(c?.clauses ?? {}).map(([k, v]) => [k, [...v.values]]))

export const conditionOf = (clauses: Clauses) => {
  const kept = Object.entries(clauses).filter(([, values]) => values.length > 0)
  return kept.length
    ? { clauses: Object.fromEntries(kept.map(([k, values]) => [k, { values }])) }
    : undefined
}

/** "Site environment is staging or qa". */
export const conditionLabel = (clauses: Clauses, types: TypeList) =>
  Object.entries(clauses)
    .map(([k, values]) => `${attributeKeyLabel(k, types)} is ${values.join(' or ')}`)
    .join(' and ')

const matches = (clauses: Clauses, attrs: Record<string, string>) =>
  Object.entries(clauses).every(([k, values]) => values.includes(attrs[k]))

/** More attributes, then narrower lists, win among Set rows. */
const specificity = (clauses: Clauses) => {
  const lists = Object.values(clauses)
  return lists.length * 1000 - lists.reduce((n, values) => n + values.length, 0)
}

// ── Consequences ──────────────────────────────────────────────────────────────

type RowLike = Pick<Entitlement, 'effect' | 'value' | 'condition' | 'level' | 'levelTypeId'>

export type Line = { label: string; value: string }

const capitalize = (s: string) => s.charAt(0).toUpperCase() + s.slice(1)

/** Who a level's value is for: "Each site", "Across all sites", "Each customer". */
export function levelSubject(key: LevelKey, f: Feature | undefined, types: Types): string {
  const label = levelLabel(key, f, types)
  return label.startsWith('Per ') ? `Each ${label.slice(4)}` : label === 'Customer' ? 'Each customer' : label
}

/** What one row gives, its condition and level aside (the line's subject names the level). */
export function rowPhrase(e: RowLike, f: Feature | undefined): string {
  const value = valueLabel(e.value, f)
  const effect = effectOf(e)
  switch (effect) {
    case EntitlementEffect.ADD:
      return `+${value} per unit bought`
    case EntitlementEffect.RAISE_TO:
    case EntitlementEffect.CAP_AT:
      return `${EFFECT[effect].label} ${value}`
    default:
      return capitalize(value)
  }
}

const levelRank = (k: LevelKey) => (k === 'feature' ? 0 : k === 'customer' ? 1 : 2)

/** One owner's rows of a feature, per level, and per attribute combination where conditions split a level. */
export function rowsLines(rows: RowLike[], f: Feature, types: Types, ownerName?: string): Line[] {
  const byLevel = new Map<LevelKey, RowLike[]>()
  rows.forEach(r => byLevel.set(levelKeyOf(r), [...(byLevel.get(levelKeyOf(r)) ?? []), r]))
  return [...byLevel.entries()]
    .sort(([a], [b]) => levelRank(a) - levelRank(b) || a.localeCompare(b))
    .flatMap(([key, group]) => {
      const subject = `${levelSubject(key, f, types)}${ownerName ? ` on ${ownerName}` : ''}`
      const table = outcomes(group, f, types.get(levelTypeId(key, f) ?? ''))
      const cells = table && table !== 'tooMany' ? table : undefined
      const perRow = cells ? group.filter(r => effectOf(r) !== EntitlementEffect.SET) : group
      return [
        ...(cells ?? []).map(c => {
          const combo = Object.fromEntries(Object.entries(c.combo).map(([k, v]) => [k, [v]]))
          return { label: `${subject} · ${conditionLabel(combo, types)}`, value: c.value }
        }),
        ...perRow.map(r => {
          const clauses = clausesOf(r.condition)
          const when = Object.keys(clauses).length ? ` · ${conditionLabel(clauses, types)}` : ''
          return { label: `${subject}${when}`, value: rowPhrase(r, f) }
        }),
      ]
    })
}

const MAX_OUTCOME_CELLS = 12

/** The winner of one owner's Set rows on a level, per combination of the attribute values they read. */
function outcomes(
  rows: RowLike[],
  f: Feature | undefined,
  type: ResourceType | undefined
): { combo: Record<string, string>; value: string }[] | 'tooMany' | undefined {
  const sets = rows.filter(r => effectOf(r) === EntitlementEffect.SET)
  const attributes = [...new Set(sets.flatMap(r => Object.keys(clausesOf(r.condition))))].sort()
  if (!type || attributes.length === 0) return undefined
  const valuesOf = (key: string) => type.attributes[key.split('.').slice(1).join('.')]?.values ?? []
  const count = attributes.reduce((n, a) => n * Math.max(valuesOf(a).length, 1), 1)
  if (count > MAX_OUTCOME_CELLS) return 'tooMany'
  const combos = attributes.reduce<Record<string, string>[]>(
    (acc, a) => acc.flatMap(c => valuesOf(a).map(v => ({ ...c, [a]: v }))),
    [{}]
  )
  return combos.map(combo => {
    const live = sets
      .map(r => ({ r, clauses: clausesOf(r.condition) }))
      .filter(({ clauses }) => matches(clauses, combo))
    if (!live.length) return { combo, value: '—' }
    const top = Math.max(...live.map(l => specificity(l.clauses)))
    const best = live
      .filter(l => specificity(l.clauses) === top)
      .sort((a, b) => (valueRank(b.r.value, f) ?? 0) - (valueRank(a.r.value, f) ?? 0))
    return { combo, value: capitalize(valueLabel(best[0].r.value, f)) }
  })
}

const isOffValue = (v: EntitlementValue) =>
  (v.value.case === 'booleanValue' || v.value.case === 'meteredValue') && !v.value.value.enabled

const isAdditive = (f: Feature) =>
  f.kind === FeatureKind.LIMIT ||
  (f.kind === FeatureKind.VALUE && f.value?.type === FeatureValueType.NUMBER && f.value.higherIsBetter !== false)

/** `base + value × quantity`: off or nothing starts from 0, unlimited absorbs. */
function added(base: EntitlementValue | undefined, v: EntitlementValue, quantity: number) {
  const inner = v.value
  try {
    if (inner.case === 'meteredValue') {
      const b = base?.value.case === 'meteredValue' ? base.value.value : undefined
      const limit =
        !inner.value.limit || (b?.enabled && !b.limit)
          ? undefined
          : new Decimal(inner.value.limit).times(quantity).plus(b?.enabled && b.limit ? b.limit : 0).toString()
      return create(EntitlementValueSchema, { value: { case: 'meteredValue', value: { enabled: true, limit } } })
    }
    if (inner.case === 'configValue' && inner.value.value.case === 'number') {
      const b = base?.value.case === 'configValue' ? base.value.value.value : undefined
      const sum = new Decimal(inner.value.value.value).times(quantity).plus(b?.case === 'number' ? b.value : 0)
      return create(EntitlementValueSchema, {
        value: { case: 'configValue', value: { value: { case: 'number', value: sum.toString() } } },
      })
    }
  } catch {
    return undefined
  }
  return undefined
}

/** 1 when `a` is better, -1 when worse, 0 when equal; unset when they can't be compared. */
const compareValues = (a: EntitlementValue, b: EntitlementValue, f: Feature) => {
  const ra = valueRank(a, f)
  const rb = valueRank(b, f)
  if (ra === undefined || rb === undefined) return undefined
  return ra === rb ? 0 : ra > rb ? 1 : -1
}

export const sameValue = (a: EntitlementValue | undefined, b: EntitlementValue | undefined, f: Feature) =>
  a?.value.case === b?.value.case && valueLabel(a, f) === valueLabel(b, f)

/** A fold's running value; `unconstrained` on a level other than the feature's own, before any row. */
type Acc = EntitlementValue | 'unconstrained' | undefined

/** One Add, Raise to or Cap at row over the running value, as the server applies it; unset when it changes nothing. */
function step(acc: Acc, v: EntitlementValue, effect: EntitlementEffect, quantity: number, f: Feature) {
  const current = acc === 'unconstrained' ? undefined : acc
  let next: EntitlementValue | undefined
  if (effect === EntitlementEffect.CAP_AT)
    next = acc === 'unconstrained' ? v : current && compareValues(v, current, f) === -1 ? v : undefined
  else if (isOffValue(v) || quantity === 0 || acc === 'unconstrained') next = undefined
  else if (effect === EntitlementEffect.ADD && isAdditive(f)) next = added(current, v, quantity)
  else next = current && compareValues(v, current, f) !== 1 ? undefined : v
  return next && current && sameValue(next, current, f) ? undefined : next
}

/** A row's value over `base` on the feature's own level, its owner held `quantity` times, as the fold applies it. */
export function applyRow(
  base: EntitlementValue | undefined,
  row: Pick<Entitlement, 'effect' | 'value'>,
  quantity: number,
  f: Feature
): EntitlementValue | undefined {
  const v = row.value
  if (!v) return base
  const effect = effectOf(row)
  if (effect === EntitlementEffect.SET) return v
  return step(base, v, effect, quantity, f) ?? base
}

/** A row of one level, live and its condition met, as the fold reads it. */
export type FoldRow = {
  key: string
  tier: Tier
  effect: EntitlementEffect
  value?: EntitlementValue
  condition?: Condition
  quantity: number
  /** A limit counted per unit: the units held, which multiply its amount. */
  units?: number
}

const TIER_SOURCE: Record<Tier, Source> = {
  [Tier.DEFAULT]: 'default',
  [Tier.PLAN]: 'plan',
  [Tier.PLAN_VERSION]: 'plan',
  [Tier.SUBSCRIPTION]: 'subscription',
  [Tier.ADD_ON]: 'addOn',
  [Tier.CUSTOMER]: 'customer',
  [Tier.RESOURCE_OVERRIDE]: 'resource',
}

const granted = (r: FoldRow & { value: EntitlementValue }): EntitlementValue => {
  const inner = r.value.value
  if (r.units === undefined || inner.case !== 'meteredValue' || !inner.value.enabled || !inner.value.limit)
    return r.value
  const limit = new Decimal(inner.value.limit).times(r.units).toString()
  return create(EntitlementValueSchema, { value: { case: 'meteredValue', value: { enabled: true, limit } } })
}

/**
 * The server's fold of one level (ADR 0011 §3.2): tier by tier, the best Set to replaces all
 * below it, then Add, Raise to and Cap at apply in turn. `applied` holds the rows that count.
 */
export function foldRows(
  rows: FoldRow[],
  f: Feature,
  ownLevel: boolean
): { value: EntitlementValue | undefined; applied: string[] } {
  let acc: Acc = ownLevel ? undefined : 'unconstrained'
  let applied: string[] = []
  const tiers = [...new Set(rows.map(r => r.tier))].sort((a, b) => a - b)
  for (const tier of tiers) {
    const allowed = allowedEffects(TIER_SOURCE[tier], isOrdered(f))
    const kept = rows.filter(
      (r): r is FoldRow & { value: EntitlementValue } =>
        r.tier === tier && !!r.value && allowed.includes(effectOf(r))
    )
    const winner = kept
      .filter(r => effectOf(r) === EntitlementEffect.SET)
      .reduce<(typeof kept)[number] | undefined>((best, r) => {
        if (!best) return r
        const bySpecificity = specificity(clausesOf(r.condition)) - specificity(clausesOf(best.condition))
        if (bySpecificity !== 0) return bySpecificity > 0 ? r : best
        return compareValues(granted(r), granted(best), f) === 1 ? r : best
      }, undefined)
    if (winner) {
      acc = granted(winner)
      applied = [winner.key]
    }
    for (const effect of [EntitlementEffect.ADD, EntitlementEffect.RAISE_TO, EntitlementEffect.CAP_AT]) {
      for (const r of kept.filter(r => effectOf(r) === effect)) {
        const next = step(acc, granted(r), effect, r.quantity, f)
        if (next) {
          acc = next
          applied.push(r.key)
        }
      }
    }
  }
  return { value: acc === 'unconstrained' ? undefined : acc, applied }
}

/** A resolved preview's value as an entitlement value, for folding rows over it. */
export const resolvedEntitlementValue = (r: ResolvedEntitlement | undefined): EntitlementValue | undefined => {
  const inner = r?.value
  switch (inner?.case) {
    case 'boolean':
      return create(EntitlementValueSchema, { value: { case: 'booleanValue', value: { enabled: inner.value.enabled } } })
    case 'metered':
      return create(EntitlementValueSchema, {
        value: { case: 'meteredValue', value: { enabled: inner.value.enabled, limit: inner.value.limit } },
      })
    case 'config':
      return create(EntitlementValueSchema, { value: { case: 'configValue', value: inner.value.value ?? {} } })
    default:
      return undefined
  }
}

/** "600 → 900 with 1, 1,200 with 2": an add-on row over a plan's value. */
export function addOnOutcome(
  base: EntitlementValue | undefined,
  row: Pick<Entitlement, 'effect' | 'value'>,
  f: Feature,
  multiInstance: boolean
): string {
  const label = (v: EntitlementValue | undefined) => (v ? capitalize(valueLabel(v)) : '—')
  const from = label(base)
  const counts = multiInstance && effectOf(row) === EntitlementEffect.ADD ? [1, 2] : [1]
  const results = counts.map(n => label(applyRow(base, row, n, f)))
  if (results.every(r => r === from)) return `${from} · unchanged`
  return counts.length > 1
    ? `${from} → ${results.map((r, i) => `${r} with ${counts[i]}`).join(', ')}`
    : `${from} → ${results[0]}`
}

// ── Policy ────────────────────────────────────────────────────────────────────

const RESETS_LABEL: Record<PolicyResets, string> = {
  [PolicyResets.UNSPECIFIED]: 'resets each billing cycle',
  [PolicyResets.BILLING_CYCLE]: 'resets each billing cycle',
  [PolicyResets.CALENDAR]: 'resets each calendar {n}',
  [PolicyResets.FIXED]: 'resets every {n}',
  [PolicyResets.SLIDING]: 'over the last {n}',
  [PolicyResets.NEVER]: 'never resets',
}

const periodLabel = (p: FeaturePolicy) => {
  const unit = CalendarUnit[p.period?.unit ?? CalendarUnit.DAY].toLowerCase()
  const interval = p.period?.interval ?? 1
  return interval === 1 ? unit : `${interval} ${unit}s`
}

export function policyLabel(p: FeaturePolicy | undefined, appCounted: boolean): string {
  if (!p) return appCounted ? 'Deny new at limit · never resets' : '—'
  const resets = appCounted ? 'never resets' : RESETS_LABEL[p.resets].replace('{n}', periodLabel(p))
  const atLimit =
    p.atLimit === PolicyAtLimit.ALLOW_AND_BILL
      ? 'Overage allowed'
      : p.atLimit === PolicyAtLimit.DENY_NEW || appCounted
        ? 'Deny new at limit'
        : p.gracePct
          ? `Deny at limit + ${p.gracePct}% grace`
          : 'Deny at limit'
  const alerts = p.thresholds.length ? `alerts at ${p.thresholds.map(t => `${t}%`).join(', ')}` : ''
  return [atLimit, resets, alerts].filter(Boolean).join(' · ')
}

export const RESETS_OPTIONS = [
  ['BILLING_CYCLE', 'Each billing cycle'],
  ['CALENDAR', 'On calendar boundaries'],
  ['FIXED', 'Every period from activation'],
  ['SLIDING', 'Over the last period'],
  ['NEVER', 'Never'],
] as const

export const PERIOD_UNITS = ['HOUR', 'DAY', 'WEEK', 'MONTH', 'YEAR'] as const

const windowed = (resets: string) => resets === 'CALENDAR' || resets === 'FIXED' || resets === 'SLIDING'

export const policySchema = z
  .object({
    resets: z.enum(['BILLING_CYCLE', 'CALENDAR', 'FIXED', 'SLIDING', 'NEVER']),
    periodUnit: z.enum(PERIOD_UNITS),
    periodInterval: z.string().optional(),
    atLimit: z.enum(['DENY', 'ALLOW_AND_BILL', 'DENY_NEW']),
    gracePct: z.string().optional(),
    thresholds: z.string(),
  })
  .superRefine((p, ctx) => {
    if (windowed(p.resets) && !/^[1-9]\d*$/.test(p.periodInterval ?? ''))
      ctx.addIssue({ code: 'custom', path: ['periodInterval'], message: 'A whole number' })
    if (p.gracePct && !/^\d+$/.test(p.gracePct))
      ctx.addIssue({ code: 'custom', path: ['gracePct'], message: 'A whole percentage' })
    try {
      parseThresholds(p.thresholds)
    } catch (error) {
      ctx.addIssue({ code: 'custom', path: ['thresholds'], message: (error as Error).message })
    }
  })

export type PolicyForm = z.infer<typeof policySchema>

export const policyToForm = (p: FeaturePolicy | undefined, appCounted: boolean): PolicyForm => ({
  resets: appCounted
    ? 'NEVER'
    : p && p.resets !== PolicyResets.UNSPECIFIED
      ? (PolicyResets[p.resets] as PolicyForm['resets'])
      : 'BILLING_CYCLE',
  periodUnit: p?.period ? (CalendarUnit[p.period.unit] as PolicyForm['periodUnit']) : 'MONTH',
  periodInterval: String(p?.period?.interval ?? 1),
  atLimit: appCounted
    ? 'DENY_NEW'
    : p?.atLimit === PolicyAtLimit.ALLOW_AND_BILL
      ? 'ALLOW_AND_BILL'
      : 'DENY',
  gracePct: p?.gracePct ? String(p.gracePct) : '',
  thresholds: formatThresholds(p?.thresholds ?? [80, 100]),
})

/** A limit counted by the app never resets and only blocks new usage, without grace. */
export const policyFromForm = (
  p: PolicyForm,
  appCounted: boolean
): MessageInitShape<typeof FeaturePolicySchema> => ({
  resets: appCounted ? PolicyResets.NEVER : PolicyResets[p.resets],
  period:
    !appCounted && windowed(p.resets)
      ? { unit: CalendarUnit[p.periodUnit], interval: Number(p.periodInterval) }
      : undefined,
  atLimit: appCounted ? PolicyAtLimit.DENY_NEW : PolicyAtLimit[p.atLimit],
  gracePct: !appCounted && p.atLimit === 'DENY' && p.gracePct ? Number(p.gracePct) : 0,
  thresholds: parseThresholds(p.thresholds),
})

// ── Explain ───────────────────────────────────────────────────────────────────

export const TIER_LABEL: Record<Tier, string> = {
  [Tier.DEFAULT]: 'Default',
  [Tier.PLAN]: 'Plan',
  [Tier.PLAN_VERSION]: 'Plan version',
  [Tier.SUBSCRIPTION]: 'Subscription',
  [Tier.ADD_ON]: 'Add-on',
  [Tier.CUSTOMER]: 'Override',
  [Tier.RESOURCE_OVERRIDE]: 'Override',
}

export const DISCARD_LABEL: Record<DiscardReason, string> = {
  [DiscardReason.UNSPECIFIED]: '',
  [DiscardReason.REPLACED_BY_HIGHER_TIER]: 'replaced by a higher tier',
  [DiscardReason.LOST_WITHIN_TIER]: 'a better row of the same tier wins',
  [DiscardReason.NO_EFFECT]: 'no effect',
  [DiscardReason.NO_BASE_ON_LEVEL]: 'discarded: no base on this level',
  [DiscardReason.EFFECT_NOT_ALLOWED]: 'effect not allowed here',
  [DiscardReason.CONDITION_NOT_MET]: 'condition not met',
  [DiscardReason.NOT_IN_EFFECT]: 'not in effect now',
}

/** Not a decision about the customer: a setup to fix, or usage not read. */
export const isWarningReason = (reason: CheckReason | undefined) =>
  reason === CheckReason.RESOURCE_REQUIRED ||
  reason === CheckReason.METRIC_NOT_FOUND ||
  reason === CheckReason.USAGE_UNAVAILABLE

/** "Needs a site" for `RESOURCE_REQUIRED`: the code stays in tooltips. */
export const reasonLabel = (reason: CheckReason | undefined, type: ResourceType | undefined) => {
  switch (reason) {
    case CheckReason.RESOURCE_REQUIRED:
      return `Needs a ${type ? lower(type.name) : 'resource'}`
    case CheckReason.NOT_SUBSCRIBED:
      return 'No subscription with it'
    case CheckReason.NOT_ENTITLED:
      return 'Not entitled'
    case CheckReason.METRIC_NOT_FOUND:
      return 'Metric not found'
    case CheckReason.USAGE_UNAVAILABLE:
      return 'Usage unavailable'
    case CheckReason.LIMIT_REACHED:
      return 'Limit reached'
    case CheckReason.BUDGET_EXCEEDED:
      return 'Budget exceeded'
    case CheckReason.CREDIT_EXHAUSTED:
      return 'Credit exhausted'
    default:
      return undefined
  }
}
