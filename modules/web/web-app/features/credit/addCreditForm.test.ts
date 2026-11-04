import { describe, expect, it } from 'vitest'

import { defaultProductScope } from '@/features/credit/ProductScopeField'
import { buildSchema, canSubmitCredit } from '@/features/credit/addCreditForm'

const filled = {
  collection: 'invoice',
  amount: '100',
  unitPrice: '',
  paymentMethodId: '',
  expiresInMonths: '',
  priority: 100,
  reason: 'Bank transfer of 12 March',
  reference: 'RCPT-42',
  note: '',
  productScope: defaultProductScope,
  subscriptionId: null,
  appliesTo: 'ALL',
}

describe('the nature of credit added by hand', () => {
  it('is never preselected: the form refuses to submit until one is chosen', () => {
    const result = buildSchema('EUR').safeParse(filled)
    expect(result.success).toBe(false)
    expect(result.error?.issues.map(issue => issue.path.join('.'))).toContain('type')
    expect(canSubmitCredit(undefined, false)).toBe(false)
  })

  it('submits once goodwill or paid outside Meteroid is chosen', () => {
    for (const type of ['promotional', 'outside'] as const) {
      expect(buildSchema('EUR').safeParse({ ...filled, type }).success).toBe(true)
      expect(canSubmitCredit(type, false)).toBe(true)
    }
  })

  it('stays disabled while a submission is in flight', () => {
    expect(canSubmitCredit('outside', true)).toBe(false)
  })
})

describe('a credit restricted to a subscription and to products', () => {
  const scoped = {
    ...filled,
    type: 'promotional' as const,
    subscriptionId: 'sub_1',
    productScope: { mode: 'selected' as const, productIds: ['prod_api'] },
  }
  const scope = () => ({ subscriptionProductIds: new Set(['prod_api']), planName: 'Pro' })

  it('accepts products the subscription bills', () => {
    expect(buildSchema('EUR', undefined, scope).safeParse(scoped).success).toBe(true)
  })

  it('refuses a product the subscription does not bill: the credit would never pay it', () => {
    const result = buildSchema('EUR', undefined, scope).safeParse({
      ...scoped,
      productScope: { mode: 'selected', productIds: ['prod_api', 'prod_storage'] },
    })
    expect(result.success).toBe(false)
    expect(result.error?.issues[0]?.path.join('.')).toBe('productScope.productIds')
    expect(result.error?.issues[0]?.message).toBe(
      'Pro does not bill every selected product: credit for it would never pay the others.'
    )
  })

  it('is not checked while the subscription products are unknown, or for every product', () => {
    expect(
      buildSchema('EUR').safeParse({
        ...scoped,
        productScope: { mode: 'selected', productIds: ['prod_storage'] },
      }).success
    ).toBe(true)
    expect(
      buildSchema('EUR', undefined, scope).safeParse({
        ...scoped,
        productScope: defaultProductScope,
      }).success
    ).toBe(true)
  })
})
