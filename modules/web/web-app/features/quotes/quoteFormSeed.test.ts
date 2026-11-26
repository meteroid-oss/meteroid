import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import {
  DetailedQuoteSchema,
  QuoteComponentSchema,
  QuoteSchema,
} from '@/rpc/api/quotes/v1/models_pb'
import {
  SubscriptionFeeBillingPeriod,
  SubscriptionFeeSchema,
} from '@/rpc/api/subscriptions/v1/models_pb'

import { quoteToFormSeed, quoteToPriceComponentsState, quoteToTemplateSeed } from './quoteFormSeed'

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

describe('the resource a quote is for', () => {
  const quote = create(DetailedQuoteSchema, {
    quote: create(QuoteSchema, { customerId: 'c1', resourceId: 'r1' }),
  })

  it('is kept when the quote is edited', () => {
    expect(quoteToFormSeed(quote, []).resourceId).toBe('r1')
  })

  it('is dropped when the quote seeds one for another deal', () => {
    const seed = quoteToTemplateSeed(quote, [], { customerId: 'c2', planVersionIsActive: true })
    expect(seed.resourceId).toBeUndefined()
  })
})
