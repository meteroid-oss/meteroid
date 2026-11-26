import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import {
  CalendarUnit,
  FeaturePolicySchema,
  PolicyAtLimit,
  PolicyResets,
} from '@/rpc/api/entitlements/v1/models_pb'

import { policyFromForm, policyLabel, policySchema, policyToForm } from './model'

describe('policy windows', () => {
  it('keep their unit and interval through the form', () => {
    const hourly = create(FeaturePolicySchema, {
      resets: PolicyResets.CALENDAR,
      period: { unit: CalendarUnit.HOUR, interval: 1 },
      atLimit: PolicyAtLimit.DENY,
      thresholds: [80],
    })
    const form = policyToForm(hourly, false)
    expect(form).toMatchObject({ resets: 'CALENDAR', periodUnit: 'HOUR', periodInterval: '1' })
    expect(policyFromForm(form, false).period).toEqual({ unit: CalendarUnit.HOUR, interval: 1 })
    expect(policyLabel(hourly, false)).toBe('Deny at limit · resets each calendar hour · alerts at 80%')
  })

  it('label several units and drop the period where it does not apply', () => {
    const sliding = create(FeaturePolicySchema, {
      resets: PolicyResets.SLIDING,
      period: { unit: CalendarUnit.WEEK, interval: 2 },
      atLimit: PolicyAtLimit.ALLOW_AND_BILL,
    })
    expect(policyLabel(sliding, false)).toBe('Overage allowed · over the last 2 weeks')
    const form = { ...policyToForm(sliding, false), resets: 'BILLING_CYCLE' as const }
    expect(policyFromForm(form, false).period).toBeUndefined()
    expect(policySchema.safeParse({ ...form, resets: 'FIXED', periodInterval: '0' }).success).toBe(false)
  })
})
