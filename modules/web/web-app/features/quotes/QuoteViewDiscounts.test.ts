import { describe, expect, it } from 'vitest'

import { couponDurationLabel } from '@/features/quotes/QuoteViewDiscounts'
import { parseRevisionDocument } from '@/features/quotes/detail/revisionDocument'

describe('quote coupon duration', () => {
  it('reads months before the periods an older coupon counts', () => {
    expect(couponDurationLabel({ durationMonths: 12 })).toBe('for 12 months')
    expect(couponDurationLabel({ durationMonths: 1 })).toBe('for 1 month')
    expect(couponDurationLabel({ recurringValue: 3 })).toBe('first 3 billing periods')
    expect(couponDurationLabel({ recurringValue: 1 })).toBe('once')
    expect(couponDurationLabel({})).toBe('forever')
  })

  it('reads months from a recorded revision', () => {
    const coupon = (terms: object) =>
      parseRevisionDocument(
        JSON.stringify({ coupons: [{ code: 'YC26', discount: { Percentage: '50' }, ...terms }] })
      )?.coupons[0]?.duration
    expect(coupon({ duration_months: 12 })).toBe('For 12 months')
    expect(coupon({ recurring_value: 3 })).toBe('First 3 billing periods')
    expect(coupon({})).toBe('Forever')
  })
})
