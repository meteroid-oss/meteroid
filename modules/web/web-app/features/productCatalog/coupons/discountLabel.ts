import { formatCurrencyNoRounding } from '@/lib/utils/numbers'
import { CouponDiscount_CreditScope } from '@/rpc/api/coupons/v1/models_pb'

import type { CouponDiscount } from '@/rpc/api/coupons/v1/models_pb'

export const CREDIT_COUPON_HINT =
  "Adds promotional credit, once per customer. It doesn't discount invoices."

export const creditExpiryLabel = (days: number | undefined) =>
  days === undefined ? 'Never expires' : `Expires after ${days} ${days === 1 ? 'day' : 'days'}`

export const couponDiscountLabel = (discount: CouponDiscount | undefined): string => {
  const discountType = discount?.discountType
  switch (discountType?.case) {
    case 'percentage':
      return `${discountType.value.percentage}% off`
    case 'fixed':
      return `${formatCurrencyNoRounding(discountType.value.amount, discountType.value.currency)} off`
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
