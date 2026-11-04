import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { formatSubscriptionFee } from '@/features/subscriptions/utils/fees'
import { CreditUnitSchema } from '@/rpc/api/creditunits/v1/models_pb'
import { PriceSchema } from '@/rpc/api/prices/v1/models_pb'

import {
  defaultNetTerms,
  formatUsagePriceSummary,
  priceToSubscriptionFee,
} from './priceToSubscriptionFee'

import type { Price } from '@/rpc/api/prices/v1/models_pb'

const apiCredits = create(CreditUnitSchema, {
  id: 'cu_api',
  code: 'API',
  name: 'API credit',
  pluralName: 'API credits',
  precision: 2,
})

const pack = (unitPrice: string, creditUnitId?: string): Price =>
  create(PriceSchema, {
    currency: 'USD',
    pricing: {
      case: 'creditPricing',
      value: { unitPrice, quantity: 1, creditAmount: '5000', creditUnitId, once: false },
    },
  })

describe('priceToSubscriptionFee: credit packs (RF-31)', () => {
  it('sets the granted credit unit, without a rate, on a unit pack', () => {
    const fee = priceToSubscriptionFee(pack('50', 'cu_api'), { creditUnits: [apiCredits] })

    expect(fee.creditUnit).toMatchObject({
      creditUnitId: 'cu_api',
      code: 'API',
      precision: 2,
      name: 'API credit',
      pluralName: 'API credits',
    })
    expect(fee.creditUnit?.rate).toBeUndefined()
    const formatted = formatSubscriptionFee(fee, 'USD', [apiCredits])
    expect(formatted.details).toBe('5,000 API credits')
    expect(formatted.amount).toBe('$50.00')
  })

  it('names the unit from the fee when the renderer is not given the tenant units', () => {
    const fee = priceToSubscriptionFee(pack('0', 'cu_api'), { creditUnits: [apiCredits] })
    expect(formatSubscriptionFee(fee, 'USD').details).toBe('5,000 API credits')
  })

  it('leaves the credit unit unset on a money pack', () => {
    expect(
      priceToSubscriptionFee(pack('50'), { creditUnits: [apiCredits] }).creditUnit
    ).toBeUndefined()
  })
})

describe('formatUsagePriceSummary: credit-priced usage', () => {
  const usage = (creditUnit?: { creditUnitId: string; rate: string }) =>
    create(PriceSchema, {
      currency: 'EUR',
      creditUnit,
      pricing: { case: 'usagePricing', value: { model: { case: 'perUnit', value: '1' } } },
    })

  it('shows credits, not money, for a price in a credit unit', () => {
    expect(
      formatUsagePriceSummary(usage({ creditUnitId: 'cu_api', rate: '0.04' }), 'EUR', [apiCredits])
        .amount
    ).toBe('1 API credit/unit')
  })

  it('adds the money each credit is billed at when not covered', () => {
    const summary = formatUsagePriceSummary(
      usage({ creditUnitId: 'cu_api', rate: '0.04' }),
      'EUR',
      [apiCredits]
    )
    expect(summary.note).toMatch(/^1 API credit = €0\.04, billed when not covered by API credits$/)
  })

  it('shows money for a money price', () => {
    expect(formatUsagePriceSummary(usage(), 'EUR').amount).toContain('€')
    expect(formatUsagePriceSummary(usage(), 'EUR').note).toBeUndefined()
  })
})

describe('defaultNetTerms (RF-32)', () => {
  it('is due on receipt when a paid pack is sold, from any component or add-on', () => {
    expect(defaultNetTerms([undefined, pack('0'), pack('50', 'cu_api')], 30)).toEqual({
      netTerms: 0,
      sellsCredit: true,
    })
  })

  it("keeps the plan's terms when only included packs are granted", () => {
    expect(defaultNetTerms([pack('0', 'cu_api')], 30)).toEqual({ netTerms: 30, sellsCredit: false })
  })
})
