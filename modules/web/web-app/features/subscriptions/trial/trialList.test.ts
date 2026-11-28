import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import {
  SubscriptionSchema,
  SubscriptionTrialSummarySchema,
  TrialConfig_Status,
} from '@/rpc/api/subscriptions/v1/models_pb'

import {
  TRIALS_ENDING_DAYS,
  onFileLabel,
  parseTrialView,
  trialListFilter,
  trialPaymentMethod,
} from './trialList'

describe('parseTrialView', () => {
  it('reads a known view and falls back to all', () => {
    expect(parseTrialView('awaiting_payment')).toBe('awaiting_payment')
    expect(parseTrialView('ending')).toBe('ending')
    expect(parseTrialView('nonsense')).toBe('all')
    expect(parseTrialView('')).toBe('all')
  })
})

describe('trialListFilter', () => {
  it('sends no trial filter for all', () => {
    expect(trialListFilter('all')).toBeUndefined()
  })

  it('maps each view to its server-side filter', () => {
    expect(trialListFilter('ending')).toEqual({
      kind: { case: 'endingWithinDays', value: TRIALS_ENDING_DAYS },
    })
    expect(trialListFilter('awaiting_decision')).toEqual({
      kind: { case: 'awaitingDecision', value: true },
    })
    expect(trialListFilter('awaiting_payment')).toEqual({
      kind: { case: 'awaitingPayment', value: true },
    })
  })
})

describe('trialPaymentMethod', () => {
  const subscription = (chargeAutomatically: boolean, hasPaymentMethod?: boolean) =>
    create(SubscriptionSchema, {
      chargeAutomatically,
      trial: create(SubscriptionTrialSummarySchema, {
        endsOn: '2026-12-15',
        status: TrialConfig_Status.ACTIVE,
        hasPaymentMethod,
      }),
    })

  it('is none for a trial charged automatically without a method', () => {
    expect(trialPaymentMethod(subscription(true, false))).toBe('none')
  })

  it('is on file for a trial charged automatically with a method', () => {
    expect(trialPaymentMethod(subscription(true, true))).toBe('on_file')
  })

  it('is invoice for a trial paid by invoice, whatever is on file', () => {
    expect(trialPaymentMethod(subscription(false, false))).toBe('invoice')
    expect(trialPaymentMethod(subscription(false, true))).toBe('invoice')
  })

  it('is undefined once the trial ended or without a trial', () => {
    expect(trialPaymentMethod(subscription(true, undefined))).toBeUndefined()
    expect(trialPaymentMethod(create(SubscriptionSchema, {}))).toBeUndefined()
  })
})

describe('onFileLabel', () => {
  const subscription = (paymentMethod?: string) =>
    create(SubscriptionSchema, {
      chargeAutomatically: true,
      trial: create(SubscriptionTrialSummarySchema, {
        endsOn: '2026-12-15',
        status: TrialConfig_Status.ACTIVE,
        hasPaymentMethod: true,
        paymentMethod,
      }),
    })

  it('names the method on file as the customer knows it', () => {
    expect(onFileLabel(subscription('Visa •••• 4242'))).toBe('Visa •••• 4242')
  })

  it('reads on file when the method cannot be named', () => {
    expect(onFileLabel(subscription())).toBe('On file')
  })
})
