import { CheckoutType } from '@/rpc/portal/checkout/v1/checkout_pb'

/** The pay button of a checkout says what paying does. */
export const checkoutSubmitLabel = (checkoutType?: CheckoutType): string => {
  switch (checkoutType) {
    case CheckoutType.TOP_UP:
      return 'Buy credit'
    case CheckoutType.PLAN_CHANGE:
      return 'Pay and change plan'
    case CheckoutType.ADDON_PURCHASE:
      return 'Pay and add'
    default:
      return 'Pay and subscribe'
  }
}

export const INVOICE_SUBMIT_LABEL = 'Pay invoice'
