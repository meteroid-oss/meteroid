import { describe, expect, it } from 'vitest'

import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'

import { headlineOf, priceLabel } from './headline'

const priced = (
  currency: string,
  optionId: string,
  amount: string | undefined,
  period: BillingPeriod | undefined,
  extra: { hasUsage?: boolean; fromLowest?: boolean } = {}
) => ({
  currency,
  optionId,
  amount,
  period,
  hasUsage: extra.hasUsage ?? false,
  fromLowest: extra.fromLowest ?? false,
})

describe('priceLabel', () => {
  it('reads the amount on its period', () => {
    expect(priceLabel(priced('EUR', 'm', '29.00', BillingPeriod.MONTHLY))).toBe('€29 / month')
    expect(priceLabel(priced('USD', 'y', '290.50', BillingPeriod.ANNUAL))).toBe('$290.50 / year')
  })

  it('says "from" for capacity levels', () => {
    expect(
      priceLabel(priced('USD', 'm', '49.00', BillingPeriod.MONTHLY, { fromLowest: true }))
    ).toBe('from $49 / month')
  })

  it('reads a plan of seats only by its seat price', () => {
    expect(
      priceLabel({ ...priced('EUR', 'm', '14.00', BillingPeriod.MONTHLY), unit: 'seat' })
    ).toBe('€14 / seat / month')
  })

  it('names free and usage-only options', () => {
    expect(priceLabel(priced('EUR', 'm', '0.00', BillingPeriod.MONTHLY))).toBe('Free')
    expect(priceLabel(priced('EUR', 'm', undefined, undefined, { hasUsage: true }))).toBe(
      'Usage-based'
    )
    expect(priceLabel(priced('EUR', 'm', undefined, BillingPeriod.MONTHLY))).toBeUndefined()
  })
})

describe('headlineOf', () => {
  const options = [
    priced('BRL', 'm', '150.00', BillingPeriod.MONTHLY),
    priced('EUR', 'm', '29.00', BillingPeriod.MONTHLY),
    priced('EUR', 'y', '290.00', BillingPeriod.ANNUAL),
    priced('USD', 'm', '32.00', BillingPeriod.MONTHLY),
  ]

  it("leads with the tenant's currency when sold there", () => {
    const headline = headlineOf(options, 'EUR')
    expect(headline?.lead.amount).toBe('29.00')
    expect(headline?.others.map(o => o.optionId)).toEqual(['y'])
    expect(headline?.currencies).toEqual(['EUR', 'BRL', 'USD'])
  })

  it('falls back to the first currency sold', () => {
    expect(headlineOf(options, 'GBP')?.lead.currency).toBe('BRL')
    expect(headlineOf([], 'EUR')).toBeUndefined()
  })
})
