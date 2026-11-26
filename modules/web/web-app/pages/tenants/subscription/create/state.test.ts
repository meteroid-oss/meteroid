import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { PriceComponentSchema } from '@/rpc/api/pricecomponents/v1/models_pb'
import { PriceSchema } from '@/rpc/api/prices/v1/models_pb'
import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'

import { effectiveSubscriptionPrices, resourceFor } from './state'

const pack = (id: string, cadence: BillingPeriod, unitPrice: string) =>
  create(PriceSchema, {
    id,
    cadence,
    currency: 'EUR',
    pricing: { case: 'creditPricing', value: { unitPrice, quantity: 1, creditAmount: '10' } },
  })

const component = create(PriceComponentSchema, {
  id: 'pc_1',
  prices: [pack('monthly', BillingPeriod.MONTHLY, '0'), pack('annual', BillingPeriod.ANNUAL, '90')],
})

const none = { removed: [], parameterized: [], overridden: [], extra: [] }

describe('effectiveSubscriptionPrices', () => {
  it('takes only the price the server resolves for a component', () => {
    const inOneOption = { ...component, prices: [component.prices[0]] }
    expect(effectiveSubscriptionPrices([inOneOption], none, [], 'EUR').map(p => p.id)).toEqual([
      'monthly',
    ])
  })

  it('counts nothing for a component whose price is still ambiguous', () => {
    expect(effectiveSubscriptionPrices([component], none, [], 'EUR')).toEqual([])
  })
})

describe('resourceFor', () => {
  it('keeps the URL resource for the URL customer only', () => {
    expect(resourceFor('cus_a', {}, 'cus_a', 'res_1')).toBe('res_1')
    expect(resourceFor('cus_b', {}, 'cus_a', 'res_1')).toBeUndefined()
    expect(resourceFor('cus_b', { customerId: 'cus_a', resourceId: 'res_1' }, 'cus_a', 'res_1')).toBeUndefined()
    expect(resourceFor('cus_a', { customerId: 'cus_a', resourceId: 'res_2' }, 'cus_a', 'res_1')).toBe('res_2')
  })
})
