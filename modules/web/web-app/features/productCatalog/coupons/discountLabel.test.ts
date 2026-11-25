import { describe, expect, it } from 'vitest'

import { couponDurationLabel } from '@/features/productCatalog/coupons/discountLabel'

describe('coupon duration', () => {
  it('reads months, once and forever', () => {
    expect(couponDurationLabel({ durationMonths: 12 })).toBe('For 12 months')
    expect(couponDurationLabel({ durationMonths: 1 })).toBe('For 1 month')
    expect(couponDurationLabel({ recurringValue: 1 })).toBe('Once')
    expect(couponDurationLabel({})).toBe('Forever')
  })

  it('keeps the invoices an older coupon counts', () => {
    expect(couponDurationLabel({ recurringValue: 6 })).toBe('First 6 invoices')
  })
})
