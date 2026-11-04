import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'
import { z } from 'zod'

import {
  CalendarUnit,
  EffectiveEntitlement_MeteredEntitlementSchema,
  EffectiveEntitlementSchema,
  OverageBehaviorSchema,
  ResetPeriodSchema,
} from '@/rpc/api/entitlements/v1/models_pb'

import { buildMeteredPolicy, checkMeteredPolicy, meteredPolicyToFields } from './creation/types'
import {
  configuredEnabled,
  effectiveValueLabel,
  entitlementValueToSpec,
  isEntitlementDisabled,
  limitAlertsLabel,
  limitBehaviorLabel,
  limitGroupLabel,
  limitTermsOf,
  resetPeriodSuffix,
} from './utils'

const block = (gracePeriodPct?: number) =>
  create(OverageBehaviorSchema, { Inner: { case: 'block', value: { gracePeriodPct } } })
const allow = () => create(OverageBehaviorSchema, { Inner: { case: 'allow', value: {} } })

const effective = (init: { enabled: boolean; limitReached: boolean }) =>
  create(EffectiveEntitlementSchema, {
    value: {
      case: 'metered',
      value: create(EffectiveEntitlement_MeteredEntitlementSchema, {
        metricId: 'm',
        limit: '100',
        consumed: '100',
        overageBehavior: block(),
        ...init,
      }),
    },
  }).value

describe('an effective entitlement at its blocking limit', () => {
  const reached = effective({ enabled: false, limitReached: true })

  it('is still switched on', () => {
    expect(configuredEnabled(reached)).toBe(true)
    expect(isEntitlementDisabled(reached)).toBe(false)
  })

  it('is pinned switched on, not as a lasting override to off', () => {
    const spec = entitlementValueToSpec(reached)
    expect(spec.value?.case).toBe('meteredValue')
    expect(spec.value?.case === 'meteredValue' && spec.value.value.enabled).toBe(true)
  })

  it('is written back without terms that act on nothing', () => {
    const unlimited = create(EffectiveEntitlementSchema, {
      value: {
        case: 'metered',
        value: create(EffectiveEntitlement_MeteredEntitlementSchema, {
          metricId: 'm',
          overageBehavior: allow(),
          thresholds: [50, 80],
          enabled: true,
        }),
      },
    }).value
    const spec = entitlementValueToSpec(unlimited, false)
    const written = spec.value?.case === 'meteredValue' ? spec.value.value : undefined
    expect(written?.enabled).toBe(false)
    expect(written?.overageBehavior?.Inner?.case).toBe('block')
    expect(written?.thresholds).toEqual([])
  })

  it('is switched off when the switch is off', () => {
    const off = effective({ enabled: false, limitReached: false })
    expect(configuredEnabled(off)).toBe(false)
    expect(isEntitlementDisabled(off)).toBe(true)
  })
})

describe('limit terms', () => {
  it('names hard and soft limits, with the grace', () => {
    expect(limitBehaviorLabel(limitTermsOf({ limit: '10', overageBehavior: block() }))).toBe(
      'Hard limit'
    )
    expect(limitBehaviorLabel(limitTermsOf({ limit: '10', overageBehavior: block(10) }))).toBe(
      'Hard limit (+10% grace)'
    )
    expect(limitBehaviorLabel(limitTermsOf({ limit: '10', overageBehavior: allow() }))).toBe(
      'Soft limit'
    )
    expect(limitBehaviorLabel(limitTermsOf({ limit: '10' }))).toBe('Hard limit')
  })

  it('says nothing without a limit', () => {
    expect(limitBehaviorLabel(limitTermsOf({ overageBehavior: allow() }))).toBeUndefined()
    expect(limitGroupLabel(limitTermsOf({ groupBy: 'user_id' }))).toBeUndefined()
    expect(limitGroupLabel(limitTermsOf({ limit: '5', groupBy: 'user_id' }))).toBe('per user_id')
  })

  it('lists the alert thresholds below 100%', () => {
    expect(limitAlertsLabel(limitTermsOf({ limit: '5', thresholds: [50, 80, 100] }))).toBe(
      'Alerts at 50%, 80%'
    )
    expect(limitAlertsLabel(limitTermsOf({ limit: '5', thresholds: [100] }))).toBeUndefined()
    expect(limitAlertsLabel(limitTermsOf({ thresholds: [50] }))).toBeUndefined()
  })

  it('anchors fixed windows on activation', () => {
    expect(
      resetPeriodSuffix(
        create(ResetPeriodSchema, {
          Inner: { case: 'fixedWindow', value: { unit: CalendarUnit.MONTH, interval: 1 } },
        })
      )
    ).toContain('from activation')
  })
})

describe('the metered policy sent to the server', () => {
  it('drops the overage terms and the thresholds without a limit', () => {
    const policy = buildMeteredPolicy({
      limit: ' ',
      resetPeriodType: 'billingCycle',
      overageType: 'allow',
      gracePct: '10',
      thresholds: '80',
    })
    expect(policy.overageBehavior.Inner).toEqual({ case: 'block', value: {} })
    expect(policy.thresholds).toEqual([])
  })

  it('drops the thresholds on a sliding window', () => {
    const policy = buildMeteredPolicy({
      limit: '200',
      resetPeriodType: 'slidingWindow',
      overageType: 'block',
      gracePct: '10',
      thresholds: '80',
    })
    expect(policy.overageBehavior.Inner).toEqual({ case: 'block', value: { gracePeriodPct: 10 } })
    expect(policy.thresholds).toEqual([])
  })

  it('keeps them with a limit', () => {
    const policy = buildMeteredPolicy({
      limit: '200',
      resetPeriodType: 'billingCycle',
      overageType: 'allow',
      thresholds: '50, 80, 95',
    })
    expect(policy.overageBehavior.Inner?.case).toBe('allow')
    expect(policy.thresholds).toEqual([50, 80, 95])
  })

  it('reads the stored thresholds back into the form', () => {
    expect(meteredPolicyToFields({ overageBehavior: allow(), thresholds: [50, 80] })).toEqual({
      overageType: 'allow',
      gracePct: '',
      thresholds: '50, 80',
    })
  })
})

describe('the thresholds checked in the form', () => {
  const check = (thresholds: string, resetPeriodType: 'billingCycle' | 'slidingWindow') => {
    const issues: unknown[] = []
    checkMeteredPolicy({ limit: '100', resetPeriodType, thresholds }, {
      addIssue: (issue: unknown) => issues.push(issue),
    } as unknown as z.RefinementCtx)
    return issues.length
  }

  it('are increasing percentages, unchecked where they are not shown', () => {
    expect(check('50, 80, 95', 'billingCycle')).toBe(0)
    expect(check('80, 50', 'billingCycle')).toBe(1)
    expect(check('120', 'billingCycle')).toBe(1)
    expect(check('120', 'slidingWindow')).toBe(0)
  })
})

describe('the value an effective entitlement shows', () => {
  const metered = (limit?: string, consumed?: string) =>
    create(EffectiveEntitlementSchema, {
      value: {
        case: 'metered',
        value: create(EffectiveEntitlement_MeteredEntitlementSchema, {
          metricId: 'm',
          limit,
          consumed,
          overageBehavior: allow(),
          enabled: true,
        }),
      },
    }).value

  it('reads the usage against the limit, the limit alone, or unlimited', () => {
    expect(effectiveValueLabel(metered('20000000', '5150000'))).toBe('5,150,000 / 20,000,000')
    expect(effectiveValueLabel(metered(undefined, '12'))).toBe('12 / ∞')
    expect(effectiveValueLabel(metered('500'))).toBe('500')
    expect(effectiveValueLabel(metered())).toBe('Unlimited')
  })
})
