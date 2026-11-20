import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { DetailedQuoteSchema, QuoteComponentSchema } from '@/rpc/api/quotes/v1/models_pb'
import {
  SubscriptionFeeBillingPeriod,
  SubscriptionFeeSchema,
} from '@/rpc/api/subscriptions/v1/models_pb'

import { quoteToPriceComponentsState } from './quoteFormSeed'

const rate = (value: string) =>
  create(SubscriptionFeeSchema, { fee: { case: 'rate', value: { rate: value } } })

describe('quoteToPriceComponentsState', () => {
  it('seeds a discounted line with its list price, not the net one', () => {
    const quote = create(DetailedQuoteSchema, {
      components: [
        create(QuoteComponentSchema, {
          name: 'Onboarding',
          period: SubscriptionFeeBillingPeriod.ONE_TIME,
          fee: rate('80'),
          listFee: rate('100'),
          discountPercent: '20',
        }),
      ],
    })
    const seeded = JSON.stringify(quoteToPriceComponentsState(quote, []).components.extra)
    expect(seeded).toContain('"100"')
    expect(seeded).not.toContain('"80"')
  })
})
