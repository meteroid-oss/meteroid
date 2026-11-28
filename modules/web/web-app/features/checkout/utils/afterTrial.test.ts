import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'
import { AfterTrialAmountSchema, AfterTrialSchema } from '@/rpc/portal/checkout/v1/models_pb'

import { afterTrialText } from './afterTrial'

const after = (...recurring: [bigint, BillingPeriod][]) =>
  create(AfterTrialSchema, {
    currency: 'EUR',
    startsOn: '2026-12-15',
    recurring: recurring.map(([amount, interval]) =>
      create(AfterTrialAmountSchema, { amount, interval })
    ),
  })

describe('afterTrialText', () => {
  it('says what the trial is followed by, and when', () => {
    expect(afterTrialText(after([4900n, BillingPeriod.MONTHLY]), false)).toMatch(
      /^then .*49\.00\/month from Dec 15, 2026$/
    )
    expect(afterTrialText(after([49000n, BillingPeriod.ANNUAL]), false)).toMatch(/\/year from /)
  })

  it('states each cadence its fees bill on', () => {
    expect(
      afterTrialText(after([49000n, BillingPeriod.ANNUAL], [4000n, BillingPeriod.MONTHLY]), false)
    ).toMatch(/^then .*490\.00\/year \+ .*40\.00\/month from Dec 15, 2026$/)
  })

  it('reads a usage plan’s recurring fees as a floor', () => {
    expect(afterTrialText(after([4900n, BillingPeriod.MONTHLY]), true)).toMatch(
      /^then from .*49\.00\/month, starting Dec 15, 2026$/
    )
    expect(afterTrialText(after(), true)).toBe('then usage-based from Dec 15, 2026')
  })

  it('says nothing without a trial or when nothing follows it', () => {
    expect(afterTrialText(undefined, false)).toBeUndefined()
    expect(afterTrialText(after(), false)).toBeUndefined()
  })
})
