import { describe, expect, it } from 'vitest'

import { creditCouponNote } from './creditCoupon'

describe('creditCouponNote', () => {
  it('says the credit arrives with the subscription, and for how long it lasts', () => {
    expect(creditCouponNote({ expiresInDays: 60 })).toBe(
      'Added to your balance once your subscription starts, and usable for 60 days.'
    )
    expect(creditCouponNote({ expiresInDays: 1 })).toBe(
      'Added to your balance once your subscription starts, and usable for 1 day.'
    )
    expect(creditCouponNote({})).toBe('Added to your balance once your subscription starts.')
  })
})
