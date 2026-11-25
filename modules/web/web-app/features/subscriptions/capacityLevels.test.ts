import { describe, expect, it } from 'vitest'

import { SubscriptionFeeBillingPeriod } from '@/rpc/api/subscriptions/v1/models_pb'

import { levelAppliesNow, levelMove, levelWords } from './capacityLevels'

describe('usage levels in words', () => {
  it('reads a money metric as an amount of the currency', () => {
    expect(
      levelWords(
        { included: 250_000n, rate: '899' },
        'EUR',
        SubscriptionFeeBillingPeriod.MONTHLY,
        { money: true }
      )
    ).toBe('Up to €250K — €899 / month')
  })

  it('reads other metrics as a count of their unit', () => {
    expect(
      levelWords(
        { included: 1_000_000n, rate: '49.5' },
        'USD',
        SubscriptionFeeBillingPeriod.YEARLY,
        { plural: 'requests' }
      )
    ).toBe('Up to 1M requests — $49.50 / year')
  })

  it('says the move and when it applies', () => {
    expect(levelMove(125_000n, 250_000n, 'EUR', { money: true })).toBe('€125K → €250K')
    expect(levelAppliesNow(125_000n, 250_000n)).toBe(true)
    expect(levelAppliesNow(250_000n, 125_000n)).toBe(false)
  })
})
