import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import {
  ContributionSchema,
  DiscardReason,
  EntitlementEffect,
  type EntitlementValue,
  EntitlementValueSchema,
  FeatureKind,
  FeatureSchema,
  LimitCountedBy,
  Tier,
} from '@/rpc/api/entitlements/v1/models_pb'
import { PlanOverviewSchema, PlanStatus, PlanType } from '@/rpc/api/plans/v1/models_pb'

import { applyRow, foldRows, valueLabel } from '../model'

import { overrideOutcomes, planNeighbours } from './overrideOutcomes'

const feature = create(FeatureSchema, {
  id: 'f',
  code: 'api_rate',
  name: 'API rate',
  kind: FeatureKind.LIMIT,
  limit: { countedBy: LimitCountedBy.EVENTS },
})
const limit = (n: string) =>
  create(EntitlementValueSchema, { value: { case: 'meteredValue', value: { enabled: true, limit: n } } })
const label = (v: EntitlementValue | undefined) => valueLabel(v, feature)

const plan = (value: string, discarded = DiscardReason.UNSPECIFIED) =>
  create(ContributionSchema, {
    entitlementId: 'plan',
    tier: Tier.PLAN_VERSION,
    effect: EntitlementEffect.SET,
    value: limit(value),
    quantity: 1,
    discarded,
  })
const addOn = (value: string, quantity: number, discarded = DiscardReason.UNSPECIFIED) =>
  create(ContributionSchema, {
    entitlementId: 'addon',
    tier: Tier.ADD_ON,
    effect: EntitlementEffect.ADD,
    value: limit(value),
    quantity,
    discarded,
  })

// Growth gives 600, the Extra throughput add-on adds 300 and is held twice.
const today = [plan('600'), addOn('300', 2)]
const neighbours = [
  { label: 'On Scale', value: limit('3000') },
  { label: 'On Starter', value: limit('300') },
]

const outcomes = (effect: EntitlementEffect, value: string, extra: Partial<Parameters<typeof overrideOutcomes>[0]> = {}) =>
  overrideOutcomes({
    feature,
    ownLevel: true,
    contributions: today,
    serverValue: limit('1200'),
    row: { effect, value: limit(value) },
    tier: Tier.CUSTOMER,
    plans: neighbours,
    ...extra,
  })

const table = (columns: ReturnType<typeof overrideOutcomes>) =>
  columns?.map(c => [c.label, label(c.without), label(c.with), c.addOnsDropped])

describe('overrideOutcomes', () => {
  it('raises the value now and on a smaller plan, not on a bigger one', () => {
    expect(table(outcomes(EntitlementEffect.RAISE_TO, '2000'))).toEqual([
      ['Now', '1,200', '2,000', 0],
      ['On Scale', '3,000', '3,000', 0],
      ['On Starter', '300', '2,000', 0],
    ])
  })

  it('caps only what goes past it', () => {
    expect(table(outcomes(EntitlementEffect.CAP_AT, '2000'))).toEqual([
      ['Now', '1,200', '1,200', 0],
      ['On Scale', '3,000', '2,000', 0],
      ['On Starter', '300', '300', 0],
    ])
  })

  it('sets the value everywhere and stops counting the add-on', () => {
    expect(table(outcomes(EntitlementEffect.SET, '2000'))).toEqual([
      ['Now', '1,200', '2,000', 1],
      ['On Scale', '3,000', '2,000', 0],
      ['On Starter', '300', '2,000', 0],
    ])
  })

  it("leaves an edited override's current version out", () => {
    const current = create(ContributionSchema, {
      entitlementId: 'ov1',
      tier: Tier.CUSTOMER,
      effect: EntitlementEffect.SET,
      value: limit('5000'),
      quantity: 1,
    })
    const columns = outcomes(EntitlementEffect.RAISE_TO, '2000', {
      contributions: [
        plan('600', DiscardReason.REPLACED_BY_HIGHER_TIER),
        addOn('300', 2, DiscardReason.REPLACED_BY_HIGHER_TIER),
        current,
      ],
      serverValue: limit('5000'),
      existingId: 'ov1',
      plans: [],
    })
    expect(table(columns)).toEqual([['Now', '1,200', '2,000', 0]])
  })

  it("keeps the customer's other overrides on another plan", () => {
    const other = create(ContributionSchema, {
      entitlementId: 'ov2',
      tier: Tier.CUSTOMER,
      effect: EntitlementEffect.CAP_AT,
      value: limit('2500'),
      quantity: 1,
      discarded: DiscardReason.NO_EFFECT,
    })
    const columns = outcomes(EntitlementEffect.RAISE_TO, '1000', {
      contributions: [...today, other],
      plans: [{ label: 'On Scale', value: limit('3000') }],
    })
    expect(table(columns)).toEqual([
      ['Now', '1,200', '1,200', 0],
      ['On Scale', '2,500', '2,500', 0],
    ])
  })

  it("skips rows not in effect and gives nothing when it can't reproduce the server's value", () => {
    const expired = create(ContributionSchema, {
      entitlementId: 'old',
      tier: Tier.CUSTOMER,
      effect: EntitlementEffect.SET,
      value: limit('9000'),
      quantity: 1,
      discarded: DiscardReason.NOT_IN_EFFECT,
    })
    expect(outcomes(EntitlementEffect.RAISE_TO, '2000', { contributions: [...today, expired] })).toBeDefined()
    expect(outcomes(EntitlementEffect.RAISE_TO, '2000', { serverValue: limit('999') })).toBeUndefined()
  })
})

describe('foldRows', () => {
  const row = (effect: EntitlementEffect, value: string, tier = Tier.CUSTOMER) => ({
    key: `${effect}-${value}`,
    tier,
    effect,
    value: limit(value),
    quantity: 1,
  })

  it('caps from nothing on another level, and raises nothing there', () => {
    expect(label(foldRows([row(EntitlementEffect.CAP_AT, '500')], feature, false).value)).toBe('500')
    expect(foldRows([row(EntitlementEffect.RAISE_TO, '500')], feature, false).value).toBeUndefined()
  })

  it('multiplies a per-unit amount by its units', () => {
    const perUnit = { ...row(EntitlementEffect.SET, '10', Tier.PLAN_VERSION), units: 5 }
    expect(label(foldRows([perUnit], feature, true).value)).toBe('50')
  })

  it('keeps the most specific Set to of a tier', () => {
    const broad = row(EntitlementEffect.SET, '900', Tier.PLAN_VERSION)
    const narrow = {
      ...row(EntitlementEffect.SET, '100', Tier.PLAN_VERSION),
      condition: { clauses: { 'site.environment': { values: ['staging'] } } },
    }
    expect(label(foldRows([broad, narrow], feature, true).value)).toBe('100')
  })

  it('applyRow adds once per unit held', () => {
    expect(label(applyRow(limit('600'), { effect: EntitlementEffect.ADD, value: limit('300') }, 2, feature))).toBe(
      '1,200'
    )
  })
})

describe('planNeighbours', () => {
  const overview = (
    id: string,
    rank: number | undefined,
    extra: { planStatus?: PlanStatus; productFamilyLocalId?: string } = {}
  ) =>
    create(PlanOverviewSchema, {
      id,
      name: id,
      productFamilyLocalId: 'line',
      selfServiceRank: rank,
      planStatus: PlanStatus.ACTIVE,
      planType: PlanType.STANDARD,
      createdAt: '2026-01-01T00:00:00Z',
      activeVersion: { id: `${id}-v1`, version: 1 },
      ...extra,
    })

  it('takes the nearest sellable plans up and down the ladder', () => {
    const plans = [
      overview('starter', 0),
      overview('legacy', 1, { planStatus: PlanStatus.INACTIVE }),
      overview('growth', 2),
      overview('scale', 3),
      overview('other-line', 4, { productFamilyLocalId: 'other' }),
    ]
    const { upgrade, downgrade } = planNeighbours(plans, 'growth')
    expect([upgrade?.id, downgrade?.id]).toEqual(['scale', 'starter'])
  })

  it('has none for a plan off the ladder', () => {
    expect(planNeighbours([overview('starter', 0), overview('custom', undefined)], 'custom')).toEqual({})
  })
})
