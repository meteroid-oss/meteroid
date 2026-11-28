import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { TrialAtEnd_Kind, TrialWithoutCard_Kind } from '@/rpc/api/plans/v1/models_pb'
import {
  FirstChargeEstimateSchema,
  SubscriptionFeeBillingPeriod,
  TrialConfigSchema,
} from '@/rpc/api/subscriptions/v1/models_pb'

import { atEndText, firstChargeText, periodSuffix, withoutCardText } from './trialFacts'

const trial = (atEnd: TrialAtEnd_Kind, withoutCard = TrialWithoutCard_Kind.AWAIT_PAYMENT) =>
  create(TrialConfigSchema, {
    durationDays: 14,
    startsOn: '2026-12-01',
    endsOn: '2026-12-15',
    atEnd: { kind: atEnd, planVersionId: atEnd === TrialAtEnd_Kind.SWITCH ? 'pv_free' : undefined },
    withoutCard: {
      kind: withoutCard,
      planVersionId: withoutCard === TrialWithoutCard_Kind.SWITCH ? 'pv_free' : undefined,
    },
  })

const naming = {
  isFreePlan: false,
  planName: (id?: string) => (id === 'pv_free' ? 'Free' : undefined),
  ownName: 'Pro',
}

describe('periodSuffix', () => {
  it('reads one recurring period, one-time fees aside', () => {
    expect(
      periodSuffix([SubscriptionFeeBillingPeriod.MONTHLY, SubscriptionFeeBillingPeriod.ONE_TIME])
    ).toBe('/month')
    expect(periodSuffix([SubscriptionFeeBillingPeriod.YEARLY])).toBe('/year')
  })

  it('reads none for mixed periods or none at all', () => {
    expect(
      periodSuffix([SubscriptionFeeBillingPeriod.MONTHLY, SubscriptionFeeBillingPeriod.YEARLY])
    ).toBe('')
    expect(periodSuffix([])).toBe('')
  })
})

describe('firstChargeText', () => {
  it('reads the amount, the period and the day', () => {
    const estimate = create(FirstChargeEstimateSchema, {
      amount: 4900n,
      currency: 'EUR',
      on: '2026-12-15',
    })
    expect(firstChargeText(estimate, [SubscriptionFeeBillingPeriod.MONTHLY])).toMatch(
      /49\.00\/month from Dec 15/
    )
  })
})

describe('atEndText', () => {
  it('names each end action', () => {
    expect(atEndText(trial(TrialAtEnd_Kind.CONVERT), naming)).toBe('Converts')
    expect(atEndText(trial(TrialAtEnd_Kind.MANUAL), naming)).toBe('Account owner decides')
    expect(atEndText(trial(TrialAtEnd_Kind.CANCEL), naming)).toBe('Cancels')
    expect(atEndText(trial(TrialAtEnd_Kind.SWITCH), naming)).toBe('Switches to Free')
  })

  it('reads a reverse trial as staying on its free plan', () => {
    expect(atEndText(trial(TrialAtEnd_Kind.CONVERT), { ...naming, isFreePlan: true, ownName: 'Free' })).toBe(
      'Stays on Free'
    )
  })
})

describe('withoutCardText', () => {
  const charged = { ...naming, chargedAutomatically: true }

  it('applies to a converting trial charged automatically only', () => {
    expect(withoutCardText(trial(TrialAtEnd_Kind.CONVERT), charged)).toBe('Awaits payment')
    expect(
      withoutCardText(trial(TrialAtEnd_Kind.CONVERT, TrialWithoutCard_Kind.SWITCH), charged)
    ).toBe('Switches to Free')
    expect(
      withoutCardText(trial(TrialAtEnd_Kind.CONVERT), { ...charged, chargedAutomatically: false })
    ).toBeUndefined()
    expect(withoutCardText(trial(TrialAtEnd_Kind.MANUAL), charged)).toBeUndefined()
  })
})
