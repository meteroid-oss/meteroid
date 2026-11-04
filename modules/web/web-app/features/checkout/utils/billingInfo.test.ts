import { describe, expect, it } from 'vitest'

import { hasCompleteBillingInformation } from './billingInfo'

const complete = {
  billingEmail: 'billing@acme.com',
  billingAddress: { line1: '1 Rue de la Paix', city: 'Paris', zipCode: '75002', country: 'FR' },
}

describe('hasCompleteBillingInformation', () => {
  it('accepts an email and a full address', () => {
    expect(hasCompleteBillingInformation(complete)).toBe(true)
  })

  it('requires a non-blank email', () => {
    expect(hasCompleteBillingInformation({ ...complete, billingEmail: undefined })).toBe(false)
    expect(hasCompleteBillingInformation({ ...complete, billingEmail: '  ' })).toBe(false)
  })

  it('requires every address part', () => {
    expect(hasCompleteBillingInformation(undefined)).toBe(false)
    expect(hasCompleteBillingInformation({ ...complete, billingAddress: undefined })).toBe(false)
    for (const field of ['line1', 'city', 'zipCode', 'country'] as const) {
      const billingAddress = { ...complete.billingAddress, [field]: ' ' }
      expect(hasCompleteBillingInformation({ ...complete, billingAddress })).toBe(false)
    }
  })
})
