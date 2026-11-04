import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { PriceComponentSchema } from '@/rpc/api/pricecomponents/v1/models_pb'
import { PriceSchema } from '@/rpc/api/prices/v1/models_pb'
import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'

import { effectiveSubscriptionPrices } from './state'

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
    expect(
      effectiveSubscriptionPrices(
        [component],
        { ...none, parameterized: [{ componentId: 'pc_1', billingPeriod: BillingPeriod.MONTHLY }] },
        [],
        'EUR'
      ).map(p => p.id)
    ).toEqual(['monthly'])
  })

  it('counts nothing for a component whose cadence is not chosen yet', () => {
    expect(effectiveSubscriptionPrices([component], none, [], 'EUR')).toEqual([])
  })
})
