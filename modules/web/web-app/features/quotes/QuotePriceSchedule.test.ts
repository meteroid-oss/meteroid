import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { contractItems, stepItems, stepStartLabel } from '@/features/quotes/QuotePriceSchedule'
import { QuoteContractValueSchema, QuotePriceStepSchema } from '@/rpc/api/quotes/v1/models_pb'
import { SubscriptionFeeBillingPeriod } from '@/rpc/api/subscriptions/v1/models_pb'

describe('quote price schedule', () => {
  it('reads a phase from its offset when the quote has no start date', () => {
    expect(stepStartLabel({ offsetMonths: 0 })).toBe('From the start')
    expect(stepStartLabel({ offsetMonths: 1 })).toBe('After 1 month')
    expect(stepStartLabel({ offsetMonths: 12 })).toBe('After 12 months')
  })

  it('states each price with its period, the seats and the minimum', () => {
    const step = create(QuotePriceStepSchema, {
      offsetMonths: 12,
      prices: [
        {
          name: 'Seats',
          price: '10.7',
          period: SubscriptionFeeBillingPeriod.MONTHLY,
          unit: 'seat',
        },
      ],
      seats: [{ code: 'seat', slots: 150 }],
      minimum: {},
      plan: 'Growth',
    })
    expect(stepItems(step, 'EUR', '19260')).toEqual([
      'Plan: Growth',
      'Seats: €10.70 per seat · Monthly',
      'seat: at least 150',
      'No minimum commitment',
      'Over this phase: €19,260.00',
    ])
  })

  it('states the contract value by year, with the commitments when they add to it', () => {
    const value = create(QuoteContractValueSchema, {
      termMonths: 24,
      months: 24,
      total: '30240',
      committedTotal: '32000',
      annual: '15120',
      years: ['14400', '15840'],
    })
    expect(contractItems(value, 'EUR')).toEqual([
      'Total contract value: €30,240.00',
      'Annual contract value: €15,120.00',
      'With minimum commitments: €32,000.00',
      'Year 1: €14,400.00',
      'Year 2: €15,840.00',
    ])
    expect(
      contractItems(create(QuoteContractValueSchema, { months: 12, total: '1200', committedTotal: '1200', annual: '1200', years: ['1200'] }), 'EUR')
    ).toEqual(['First 12 months: €1,200.00'])
  })

  it('reads a fee its quote no longer lists by its price alone', () => {
    const step = create(QuotePriceStepSchema, {
      offsetMonths: 0,
      prices: [{ name: '', price: '12' }],
    })
    expect(stepItems(step, 'EUR')).toEqual(['€12.00'])
  })
})
