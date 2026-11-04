import { describe, expect, it } from 'vitest'

import { CheckoutType } from '@/rpc/portal/checkout/v1/checkout_pb'

import { checkoutSubmitLabel } from './submitLabel'

describe('checkoutSubmitLabel', () => {
  it('never asks to subscribe when nothing is subscribed to', () => {
    expect(checkoutSubmitLabel(CheckoutType.TOP_UP)).toBe('Buy credit')
    expect(checkoutSubmitLabel(CheckoutType.PLAN_CHANGE)).toBe('Pay and change plan')
    expect(checkoutSubmitLabel(CheckoutType.ADDON_PURCHASE)).toBe('Pay and add')
  })

  it('keeps "Pay and subscribe" for a new subscription', () => {
    expect(checkoutSubmitLabel(CheckoutType.SELF_SERVE)).toBe('Pay and subscribe')
    expect(checkoutSubmitLabel(CheckoutType.SUBSCRIPTION_ACTIVATION)).toBe('Pay and subscribe')
  })
})
