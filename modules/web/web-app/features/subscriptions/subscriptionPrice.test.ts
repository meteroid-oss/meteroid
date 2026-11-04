import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import {
  SubscriptionFeeBillingPeriod,
  SubscriptionFeeSchema,
} from '@/rpc/api/subscriptions/v1/models_pb'

import { monthlyPackCredit, summarizeSubscriptionPrice } from './subscriptionPrice'

import type { PricedFee } from './subscriptionPrice'

const pack = (price: string, credit: string, once: boolean): PricedFee => ({
  period: SubscriptionFeeBillingPeriod.MONTHLY,
  fee: create(SubscriptionFeeSchema, {
    fee: {
      case: 'credit',
      value: {
        rate: price,
        quantity: 1,
        total: price,
        creditAmount: credit,
        creditTotal: credit,
        once,
      },
    },
  }),
})

const scale = pack('500', '550', false)
const welcome = pack('50', '50', true)

describe('summarizeSubscriptionPrice', () => {
  it('keeps a once pack out of the recurring price and lists it once at start', () => {
    const summary = summarizeSubscriptionPrice({ currency: 'USD', fees: [scale, welcome] })
    expect(summary.headline).toBe('$500.00 / month')
    expect(summary.details).toEqual([
      'includes $550.00 of credit',
      'plus $50.00 of credit once at start',
    ])
    expect([summary.headline, summary.details[0]].join(' · ')).toBe(
      '$500.00 / month · includes $550.00 of credit'
    )
  })
})

describe('summarizeSubscriptionPrice, pay as you go', () => {
  const usage: PricedFee = {
    period: SubscriptionFeeBillingPeriod.MONTHLY,
    fee: create(SubscriptionFeeSchema, {
      fee: { case: 'usage', value: { metricId: 'm1', model: { case: 'perUnit', value: '2' } } },
    }),
  }

  it('reads as prepaid credit when usage stops once credit runs out', () => {
    expect(summarizeSubscriptionPrice({ currency: 'USD', fees: [usage], prepaid: true })).toEqual({
      headline: 'Prepaid credit',
      details: ['usage billed from credit'],
    })
    expect(summarizeSubscriptionPrice({ currency: 'USD', fees: [usage] }).headline).toBe(
      'Usage-based'
    )
  })
})

describe('monthlyPackCredit', () => {
  it('counts recurring packs only', () => {
    expect(monthlyPackCredit([scale, welcome], 'USD', 'USD')?.toString()).toBe('550')
    expect(monthlyPackCredit([welcome], 'USD', 'USD')).toBeUndefined()
  })
})
