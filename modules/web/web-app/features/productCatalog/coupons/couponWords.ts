import { format } from 'date-fns'

import { eligibilitySummary } from '@/features/eligibility/eligibility'

import { couponDiscountLabel, couponDurationLabel } from './discountLabel'

import type { Coupon } from '@/rpc/api/coupons/v1/models_pb'

// A coupon's list row in plain words: what it takes off, who redeems it, what it applies to.

export type CouponStatus = 'Active' | 'Disabled' | 'Archived' | 'Exhausted' | 'Expired' | 'Scheduled'

export const couponStatus = (
  coupon: Pick<
    Coupon,
    'disabled' | 'archivedAt' | 'redemptionCount' | 'redemptionLimit' | 'expiresAt' | 'validFrom'
  >,
  now = Date.now()
): CouponStatus => {
  if (coupon.archivedAt) return 'Archived'
  if (coupon.disabled) return 'Disabled'
  if (coupon.redemptionLimit && coupon.redemptionCount >= coupon.redemptionLimit) return 'Exhausted'
  if (coupon.expiresAt && new Date(coupon.expiresAt).getTime() < now) return 'Expired'
  if (coupon.validFrom && new Date(coupon.validFrom).getTime() > now) return 'Scheduled'
  return 'Active'
}

/** "50% off · for 12 months"; a credit coupon's own terms. */
export const couponDiscountLine = (coupon: Pick<Coupon, 'discount' | 'recurringValue' | 'durationMonths'>) =>
  coupon.discount?.discountType?.case === 'credit'
    ? couponDiscountLabel(coupon.discount)
    : `${couponDiscountLabel(coupon.discount)} · ${couponDurationLabel(coupon).toLowerCase()}`

/** "Everyone · until 30 Jun 2027", "Customers in Brazil whose student is verified". */
export const couponWhoLine = (
  coupon: Pick<Coupon, 'eligibility' | 'validFrom' | 'expiresAt'>,
  country: (code: string) => string = code => code
) => {
  const who = eligibilitySummary(coupon.eligibility, country).replace(/^Only customers/, 'Customers')
  const day = (value: string) => format(new Date(value), 'd MMM yyyy')
  return [
    who,
    coupon.validFrom && `from ${day(coupon.validFrom)}`,
    coupon.expiresAt && `until ${day(coupon.expiresAt)}`,
  ]
    .filter(Boolean)
    .join(' · ')
}

const count = (n: number, one: string) => `${n} ${n === 1 ? one : `${one}s`}`

/** "2 plans · 1 add-on"; "All plans and add-ons" when unscoped. */
export const couponAppliesTo = (coupon: Pick<Coupon, 'planIds' | 'addOnIds'>) => {
  const { planIds, addOnIds } = coupon
  if (!planIds.length && !addOnIds.length) return 'All plans and add-ons'
  return [planIds.length && count(planIds.length, 'plan'), addOnIds.length && count(addOnIds.length, 'add-on')]
    .filter(Boolean)
    .join(' · ')
}
