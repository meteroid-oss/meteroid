import { addMonths, startOfDay } from 'date-fns'
import { z } from 'zod'

import {
  clausesOf,
  conditionOf,
  effectOf,
  type LevelKey,
  levelFields,
  levelKeyOf,
  policyFromForm,
  policySchema,
  policyToForm,
  isAppCounted,
} from '@/features/entitlements/model'
import {
  EntitlementEffect,
  type Entitlement,
  type EntitlementValue,
  type EntitlementValueSchema,
  type Feature,
  FeatureKind,
  FeatureValueType,
} from '@/rpc/api/entitlements/v1/models_pb'

import type { MessageInitShape } from '@bufbuild/protobuf'

export const ENDS_OPTIONS = [
  ['never', 'Never'],
  ['1m', 'After a month'],
  ['3m', 'After a quarter'],
  ['12m', 'After a year'],
  ['date', 'On a date…'],
] as const

export type Ends = (typeof ENDS_OPTIONS)[number][0]

const ENDS_MONTHS: Partial<Record<Ends, number>> = { '1m': 1, '3m': 3, '12m': 12 }

/** The end the form stands for, at the start of its calendar day; shortcuts count from its start. */
export function untilOf(
  d: { ends: Ends; validUntil?: Date; validFrom?: Date },
  today = new Date()
): Date | undefined {
  if (d.ends === 'date') return d.validUntil && startOfDay(d.validUntil)
  const months = ENDS_MONTHS[d.ends]
  return months === undefined ? undefined : addMonths(startOfDay(d.validFrom ?? today), months)
}

export const rowSchema = z
  .object({
    featureId: z.string().min(1, 'Pick a feature'),
    level: z.string(),
    effect: z.string(),
    // On/off and limits: 'on' / 'off', or a limit's 'amount' / 'unlimited'.
    mode: z.enum(['on', 'off', 'amount', 'unlimited']),
    amount: z.string().optional(),
    applies: z.enum(['always', 'when']),
    clauses: z.record(z.string(), z.array(z.string())),
    validFrom: z.date().optional(),
    ends: z.enum(['never', '1m', '3m', '12m', 'date']),
    validUntil: z.date().optional(),
    reason: z.string().optional(),
    overridePolicy: z.boolean(),
    policy: policySchema,
  })
  .superRefine((d, ctx) => {
    if (d.applies === 'when' && !conditionOf(d.clauses))
      ctx.addIssue({ code: 'custom', path: ['applies'], message: 'Pick at least one value' })
    if (d.ends === 'date' && !d.validUntil)
      ctx.addIssue({ code: 'custom', path: ['validUntil'], message: 'Pick a date' })
    const until = untilOf(d)
    if (until && d.validFrom && until <= startOfDay(d.validFrom))
      ctx.addIssue({
        code: 'custom',
        path: [d.ends === 'date' ? 'validUntil' : 'ends'],
        message: 'Must be after the start',
      })
  })

export type RowForm = z.infer<typeof rowSchema>

const isNumber = (s: string | undefined) => !!s?.trim() && Number.isFinite(Number(s))

/** The value field's error for this feature, if any. */
export function valueIssue(f: Feature, d: RowForm): string | undefined {
  if (f.kind === FeatureKind.LIMIT)
    return d.mode === 'amount' && !isNumber(d.amount) ? 'Enter a number' : undefined
  if (f.kind !== FeatureKind.VALUE) return undefined
  if (f.value?.type === FeatureValueType.NUMBER && !isNumber(d.amount)) return 'Enter a number'
  if (f.value?.type === FeatureValueType.SELECT && !d.amount) return 'Choose a value'
  if (f.value?.type === FeatureValueType.JSON) {
    try {
      JSON.parse(d.amount ?? '')
    } catch {
      return 'Must be valid JSON'
    }
  }
  return undefined
}

export function valueFromForm(
  f: Feature,
  d: RowForm
): MessageInitShape<typeof EntitlementValueSchema> {
  if (f.kind === FeatureKind.BOOLEAN)
    return { value: { case: 'booleanValue', value: { enabled: d.mode !== 'off' } } }
  if (f.kind === FeatureKind.LIMIT)
    return {
      value: {
        case: 'meteredValue',
        value: {
          limit: d.mode === 'amount' ? d.amount?.trim() : undefined,
          enabled: d.mode !== 'off',
        },
      },
    }
  const raw = d.amount ?? ''
  const value =
    f.value?.type === FeatureValueType.NUMBER
      ? ({ case: 'number', value: raw.trim() } as const)
      : f.value?.type === FeatureValueType.JSON
        ? ({ case: 'json', value: raw.trim() } as const)
        : ({ case: 'text', value: raw } as const)
  return { value: { case: 'configValue', value: { value } } }
}

function valueToForm(v: EntitlementValue | undefined): Pick<RowForm, 'mode' | 'amount'> {
  const inner = v?.value
  switch (inner?.case) {
    case 'booleanValue':
      return { mode: inner.value.enabled ? 'on' : 'off' }
    case 'meteredValue':
      return !inner.value.enabled
        ? { mode: 'off' }
        : inner.value.limit
          ? { mode: 'amount', amount: inner.value.limit }
          : { mode: 'unlimited' }
    case 'configValue': {
      const c = inner.value.value
      return { mode: 'on', amount: c.case === undefined ? '' : String(c.value) }
    }
    default:
      return { mode: 'on', amount: '' }
  }
}

// Validity bounds are stored as the UTC midnight of a calendar day; the pickers show that day locally.
const dayOf = (rfc?: string) => {
  if (!rfc) return undefined
  const [y, m, d] = rfc.slice(0, 10).split('-').map(Number)
  return new Date(y, m - 1, d)
}
const rfcOf = (date?: Date) =>
  date
    ? new Date(Date.UTC(date.getFullYear(), date.getMonth(), date.getDate())).toISOString()
    : undefined

export const rowToForm = (
  f: Feature | undefined,
  e: Entitlement | undefined,
  defaults: { level: LevelKey; effect: EntitlementEffect }
): RowForm => {
  const clauses = clausesOf(e?.condition)
  return {
    featureId: f?.id ?? '',
    level: e ? levelKeyOf(e) : defaults.level,
    effect: EntitlementEffect[e ? effectOf(e) : defaults.effect],
    ...(e
      ? valueToForm(e.value)
      : f?.kind === FeatureKind.LIMIT
        ? { mode: 'amount' as const, amount: '' }
        : {
            mode: 'on' as const,
            amount: f?.value?.type === FeatureValueType.SELECT ? f.value.options[0] : '',
          }),
    applies: Object.keys(clauses).length ? 'when' : 'always',
    clauses,
    validFrom: dayOf(e?.validFrom),
    ends: e?.validUntil ? 'date' : 'never',
    validUntil: dayOf(e?.validUntil),
    reason: e?.reason ?? '',
    overridePolicy: !!e?.policyOverride,
    policy: policyToForm(e?.policyOverride ?? f?.policy, isAppCounted(f)),
  }
}

export const effectFromForm = (d: Pick<RowForm, 'effect'>) =>
  EntitlementEffect[d.effect as keyof typeof EntitlementEffect]

/** The policy override, only on Set to rows of a limit. */
const overrideFromForm = (f: Feature, d: RowForm) =>
  f.kind === FeatureKind.LIMIT && d.overridePolicy && effectFromForm(d) === EntitlementEffect.SET
    ? policyFromForm(d.policy, isAppCounted(f))
    : undefined

export const conditionFromForm = (d: Pick<RowForm, 'applies' | 'clauses'>) =>
  d.applies === 'when' ? conditionOf(d.clauses) : undefined

/** The form without its validity window and reason, for sources that don't carry them. */
export const withoutWindow = (d: RowForm): RowForm => ({
  ...d,
  validFrom: undefined,
  ends: 'never',
  validUntil: undefined,
  reason: '',
})

export const specFromForm = (f: Feature, d: RowForm) => ({
  featureId: f.id,
  value: valueFromForm(f, d),
  effect: effectFromForm(d),
  ...levelFields(d.level as LevelKey),
  condition: conditionFromForm(d),
  validFrom: rfcOf(d.validFrom),
  validUntil: rfcOf(untilOf(d)),
  reason: d.reason?.trim() || undefined,
  policyOverride: overrideFromForm(f, d),
})

const sameDay = (rfc: string | undefined, date: Date | undefined) =>
  !!rfc && !!date && rfc.slice(0, 10) === rfcOf(date)?.slice(0, 10)

/** An edit: unset fields keep their value, so removals send their `clear_*` flag. */
export function updateFromForm(f: Feature, e: Entitlement, d: RowForm) {
  const spec = specFromForm(f, d)
  const until = untilOf(d)
  return {
    id: e.id,
    value: spec.value,
    effect: spec.effect,
    condition: spec.condition,
    clearCondition: !spec.condition && !!e.condition,
    // An unchanged day keeps the stored instant rather than its midnight.
    validFrom: sameDay(e.validFrom, d.validFrom) ? e.validFrom : spec.validFrom,
    clearValidFrom: !spec.validFrom && !!e.validFrom,
    validUntil: sameDay(e.validUntil, until) ? e.validUntil : spec.validUntil,
    clearValidUntil: !spec.validUntil && !!e.validUntil,
    reason: spec.reason,
    clearReason: !spec.reason && !!e.reason,
    policyOverride: spec.policyOverride,
    clearPolicyOverride: !spec.policyOverride && !!e.policyOverride,
  }
}
