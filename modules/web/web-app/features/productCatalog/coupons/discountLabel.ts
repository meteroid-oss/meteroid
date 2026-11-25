import { formatCurrencyNoRounding } from '@/lib/utils/numbers'
import { CouponDiscount_CreditScope } from '@/rpc/api/coupons/v1/models_pb'

import type { Coupon, CouponDiscount } from '@/rpc/api/coupons/v1/models_pb'

export const CREDIT_COUPON_HINT =
  "Adds promotional credit and doesn't discount invoices. A credit coupon is redeemed once per customer."

export const creditExpiryLabel = (days: number | undefined) =>
  days === undefined ? 'Never expires' : `Expires after ${days} ${days === 1 ? 'day' : 'days'}`

export const couponDiscountLabel = (discount: CouponDiscount | undefined): string => {
  const discountType = discount?.discountType
  switch (discountType?.case) {
    case 'percentage':
      return `${discountType.value.percentage}% off`
    case 'fixed': {
      const { amount, currency, amounts } = discountType.value
      const others = amounts.filter(a => a.currency !== currency).length
      const first = `${formatCurrencyNoRounding(amount, currency)} off`
      return others ? `${first} · +${others} ${others === 1 ? 'currency' : 'currencies'}` : first
    }
    case 'credit': {
      const { amount, currency, expiresInDays, scope } = discountType.value
      const credit = `${formatCurrencyNoRounding(amount, currency)} credit`
      const parts = [
        credit,
        expiresInDays !== undefined && creditExpiryLabel(expiresInDays).toLowerCase(),
        scope === CouponDiscount_CreditScope.SUBSCRIPTION && 'subscription only',
      ].filter(Boolean)
      return parts.join(', ')
    }
    default:
      return 'Discount'
  }
}

/**
 * How long it discounts a subscription: "Once", "Forever", "For 12 months" (the billing periods
 * starting within 12 months of the first paid period), or "First 3 invoices" for a coupon made
 * before durations in months, which still counts invoices.
 */
export const couponDurationLabel = ({
  recurringValue,
  durationMonths,
}: Pick<Coupon, 'recurringValue' | 'durationMonths'>): string => {
  if (durationMonths) return `For ${durationMonths} ${durationMonths === 1 ? 'month' : 'months'}`
  if (recurringValue === 1) return 'Once'
  if (recurringValue) return `First ${recurringValue} invoices`
  return 'Forever'
}
