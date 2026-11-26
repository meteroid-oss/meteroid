import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import {
  CalendarUnit,
  EffectiveEntitlement_MeteredEntitlementSchema,
  EffectiveEntitlementSchema,
  OverageBehaviorSchema,
  ResetPeriodSchema,
} from '@/rpc/api/entitlements/v1/models_pb'

import {
  configuredEnabled,
  effectiveValueLabel,
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

  it('is switched off when the switch is off', () => {
    const off = effective({ enabled: false, limitReached: false })
    expect(configuredEnabled(off)).toBe(false)
    expect(isEntitlementDisabled(off)).toBe(true)
  })
})

describe('limit terms', () => {
  it('says whether the limit denies or allows overage, with the grace', () => {
    expect(limitBehaviorLabel(limitTermsOf({ limit: '10', overageBehavior: block() }))).toBe(
      'Deny at limit'
    )
    expect(limitBehaviorLabel(limitTermsOf({ limit: '10', overageBehavior: block(10) }))).toBe(
      'Deny at limit + 10% grace'
    )
    expect(limitBehaviorLabel(limitTermsOf({ limit: '10', overageBehavior: allow() }))).toBe(
      'Overage allowed'
    )
    expect(limitBehaviorLabel(limitTermsOf({ limit: '10' }))).toBe('Deny at limit')
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
