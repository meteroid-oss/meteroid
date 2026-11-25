import { describe, expect, it } from 'vitest'

import { couponTermsSummary, fixedDiscountOf, otherAmountsOf } from './couponTerms'

describe('fixed amounts per currency', () => {
  it('sends the main amount, the other currencies beside it', () => {
    expect(
      fixedDiscountOf('20', 'USD', [
        { currency: 'EUR', amount: ' 18 ' },
        { currency: 'USD', amount: '20' },
        { currency: '', amount: '' },
      ])
    ).toEqual({ amount: '20', currency: 'USD', amounts: [{ currency: 'EUR', amount: '18' }] })
  })

  it('reads a saved discount back as its other currencies', () => {
    expect(
      otherAmountsOf({
        currency: 'USD',
        amounts: [
          { currency: 'USD', amount: '20' },
          { currency: 'BRL', amount: '99' },
        ] as never,
      })
    ).toEqual([{ currency: 'BRL', amount: '99' }])
  })
})

describe('couponTermsSummary', () => {
  const NAMES: Record<string, string> = { BR: 'Brazil' }
  const name = (code: string) => NAMES[code] ?? code

  it('says nothing when no condition is set', () => {
    expect(couponTermsSummary({ otherAmounts: [], addOnCount: 0 })).toBe('')
  })

  it('reads every condition set in one line', () => {
    expect(
      couponTermsSummary(
        {
          validFrom: new Date(2026, 11, 1),
          otherAmounts: [{ currency: 'EUR', amount: '10' }],
          eligibility: { countries: ['BR'], properties: [] },
        },
        name
      )
    ).toBe('From 1 Dec · EUR 10 · Brazil only')
  })

  it('reads the limits and the credit terms', () => {
    expect(
      couponTermsSummary({
        expiresAt: new Date(2027, 0, 3),
        redemptionLimit: 100,
        reusable: true,
        creditExpiresInDays: 30,
        creditSubscriptionOnly: true,
      })
    ).toBe(
      'Until 3 Jan · Max 100 redemptions · Reusable · Credit expires after 30 days · One subscription only'
    )
  })

  it('reads a cleared number input as unset', () => {
    expect(
      couponTermsSummary({
        redemptionLimit: '' as unknown as number,
        creditExpiresInDays: '' as unknown as number,
        reusable: false,
      })
    ).toBe('')
  })

  it('names the properties a rule asks', () => {
    expect(
      couponTermsSummary(
        {
          addOnCount: 2,
          eligibility: {
            countries: ['BR'],
            properties: [{ key: 'student', values: ['verified'] }],
          },
        },
        name
      )
    ).toBe('2 add-ons · Only customers in Brazil whose student is verified')
  })
})
