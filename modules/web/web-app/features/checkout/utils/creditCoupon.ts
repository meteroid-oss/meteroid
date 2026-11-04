import type { PromotionalCreditCoupon } from '@/rpc/portal/checkout/v1/models_pb'

/** When a promo code's credit arrives: it is funded once the subscription starts, not on this payment. */
export const creditCouponNote = ({
  expiresInDays,
}: Pick<PromotionalCreditCoupon, 'expiresInDays'>) =>
  expiresInDays
    ? `Added to your balance once your subscription starts, and usable for ${expiresInDays} ${expiresInDays === 1 ? 'day' : 'days'}.`
    : 'Added to your balance once your subscription starts.'
