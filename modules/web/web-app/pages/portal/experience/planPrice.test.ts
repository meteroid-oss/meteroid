import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { SubscriptionFeeBillingPeriod } from '@/rpc/api/subscriptions/v1/models_pb'
import { PlanPriceSchema } from '@/rpc/portal/subscription/v1/subscription_pb'

import {
  headlineSuffix,
  isFreePlan,
  isPayAsYouGo,
  perCadence,
  planPriceAmount,
  planPriceLines,
  planPriceShort,
} from './planPrice'

const usageRates = { minRate: '2', maxRate: '60', currency: 'USD', unitSize: 1_000_000n }

const price = (fields: { amount?: bigint; minimum?: bigint }) =>
  create(PlanPriceSchema, {
    currency: 'USD',
    cadence: SubscriptionFeeBillingPeriod.MONTHLY,
    usageBased: true,
    usageRates,
    amount: 0n,
    ...fields,
  })

describe('plan price headline', () => {
  it('headlines a minimum-only plan by its minimum, never as pay as you go', () => {
    const enterprise = price({ minimum: 500_000n })
    expect(isPayAsYouGo(enterprise)).toBe(false)
    expect(isFreePlan(enterprise)).toBe(false)
    expect(planPriceAmount(enterprise)).toBe('$5,000.00')
    expect(headlineSuffix(enterprise)).toBe('/ month minimum')
    expect(perCadence(enterprise)).toBe('minimum per month')
    expect(planPriceShort(enterprise)).toBe('$5,000 / month minimum')
    expect(planPriceLines(enterprise)).toEqual(['$2.00 – $60.00 per 1M units'])
  })

  it('keeps a usage-only plan without a minimum as pay as you go', () => {
    const payg = price({})
    expect(isPayAsYouGo(payg)).toBe(true)
    expect(isFreePlan(payg)).toBe(false)
  })
})

describe('credit unit wording', () => {
  const fastRequest = {
    code: 'FAST_REQ',
    name: 'fast request',
    pluralName: 'fast requests',
    precision: 0,
  }
  const plan = (includedAmount: bigint, rolloverMax: bigint) =>
    create(PlanPriceSchema, {
      currency: 'EUR',
      cadence: SubscriptionFeeBillingPeriod.MONTHLY,
      usageBased: true,
      amount: 2000n,
      usageRates: {
        minRate: '1',
        maxRate: '1',
        currency: 'EUR',
        unitSize: 1n,
        unitLabel: { singular: 'request', plural: 'requests' },
        creditUnit: fastRequest,
      },
      includedCredits: [
        {
          amount: includedAmount,
          currency: 'FAST_REQ',
          creditUnit: fastRequest,
          cadence: SubscriptionFeeBillingPeriod.MONTHLY,
          expiry: { periodEnd: true },
          rollover: { maxAmount: rolloverMax, expiry: { months: 1 } },
        },
      ],
    })

  it('names one credit in the singular and agrees the verb with the count', () => {
    expect(planPriceLines(plan(500n, 250n), undefined, { creditTerms: true })).toEqual([
      'Includes 500 fast requests every month',
      'Up to 250 unused fast requests roll over for 1 month',
      '+ usage at 1 fast request per request',
    ])
    expect(planPriceLines(plan(1n, 1n), undefined, { creditTerms: true }).slice(0, 2)).toEqual([
      'Includes 1 fast request every month',
      'Up to 1 unused fast request rolls over for 1 month',
    ])
  })
})
