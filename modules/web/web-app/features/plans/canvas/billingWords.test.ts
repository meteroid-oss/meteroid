import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { FeeStructure_BillingType, PriceSchema } from '@/rpc/api/prices/v1/models_pb'
import { ProductSchema } from '@/rpc/api/products/v1/models_pb'
import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'

import { billedWords, chipParts, countsWords, money, priceWords, rowWords } from './billingWords'

const { MONTHLY, ANNUAL } = BillingPeriod

const price = (pricing: object, cadence = MONTHLY, currency = 'USD') =>
  create(PriceSchema, { id: `${currency}-${cadence}`, currency, cadence, pricing: pricing as never })
const rate = (amount: string) => ({ case: 'ratePricing', value: { rate: amount } })
const usage = (model: object) => ({ case: 'usagePricing', value: { model } })
const rows = (...tiers: [bigint, string][]) =>
  tiers.map(([firstUnit, unitPrice]) => ({ firstUnit, unitPrice }))
const fee = (billingType: FeeStructure_BillingType) => ({
  product: create(ProductSchema, {
    feeStructure: { structure: { case: 'extraRecurring', value: { billingType } } },
  }),
})
const tokens = { singular: 'token', plural: 'tokens', conversionFactor: 1_000_000 }

describe('money', () => {
  it('shows cents only when the amount has some', () => {
    expect(money('USD')('99')).toBe('$99')
    expect(money('USD')('1100')).toBe('$1,100')
    expect(money('USD')('12.5')).toBe('$12.50')
    expect(money('USD')('0.0004')).toBe('$0.0004')
  })
})

describe('priceWords spells the price', () => {
  it('a flat fee per period', () => {
    expect(priceWords(price(rate('99')))).toBe('$99 / month')
    expect(priceWords(price(rate('950'), ANNUAL))).toBe('$950 / year')
  })

  it('a free first tier reads as an allowance, never just "tiered"', () => {
    const tiered = usage({ case: 'tiered', value: { rows: rows([0n, '0'], [1_000_000n, '0.0004']) } })
    expect(priceWords(price(tiered))).toBe('first 1M free, then $0.0004 per unit')
  })

  it('every graduated tier', () => {
    const tiered = usage({
      case: 'tiered',
      value: { rows: rows([0n, '0.001'], [1_000_000n, '0.0008'], [10_000_000n, '0.0005']) },
    })
    expect(priceWords(price(tiered))).toBe(
      'first 1M at $0.001, next 9M at $0.0008, then $0.0005 per unit'
    )
  })

  it('volume tiers price every unit at the rate reached', () => {
    const tiers = usage({ case: 'volume', value: { rows: rows([0n, '0.001'], [1_000_000n, '0.0008']) } })
    expect(priceWords(price(tiers))).toBe('$0.001 per unit, $0.0008 from 1M, on every unit')
  })

  it('per 1M, from the metric unit or a package', () => {
    expect(priceWords(price(usage({ case: 'perUnit', value: '3' })), { unit: tokens })).toBe(
      '$3 per 1M tokens'
    )
    const pack = usage({ case: 'package', value: { packagePrice: '40', blockSize: 1_000_000n } })
    expect(priceWords(price(pack))).toBe('$40 per 1M units')
  })

  it('a seat price per seat and period', () => {
    const seats = price({ case: 'slotPricing', value: { unitRate: '12', minSlots: 3 } })
    expect(priceWords(seats)).toBe('$12 per seat / month')
    expect(countsWords(seats)).toBe('Per seat, 3 minimum')
  })

  it('a one-time fee as its total, its tiers spelled under the name', () => {
    const onboarding = price({
      case: 'oneTimePricing',
      value: {
        unitPrice: '0',
        quantity: 12,
        model: { model: { case: 'tiered', value: { rows: rows([0n, '100'], [10n, '50']) } } },
      },
    })
    expect(priceWords(onboarding)).toBe('$1,100 once')
    expect(countsWords(onboarding)).toBe('One-time fee, 12 units: first 10 at $100, then $50 each')
  })
})

describe('billedWords says when it is invoiced', () => {
  it('once, upfront or in arrears', () => {
    expect(billedWords(price({ case: 'oneTimePricing', value: { unitPrice: '5000', quantity: 1 } }))).toBe(
      'billed once'
    )
    expect(billedWords(price(rate('12000'), ANNUAL))).toBe('billed yearly, upfront')
    expect(billedWords(price(usage({ case: 'perUnit', value: '3' })))).toBe(
      'billed monthly, in arrears'
    )
  })

  it("a fee follows its product's billing type", () => {
    const fixed = price({ case: 'extraRecurringPricing', value: { unitPrice: '50', quantity: 1 } })
    expect(billedWords(fixed, fee(FeeStructure_BillingType.ARREAR))).toBe(
      'billed monthly, in arrears'
    )
    expect(billedWords(fixed, fee(FeeStructure_BillingType.ADVANCE))).toBe(
      'billed monthly, upfront'
    )
  })
})

describe('rowWords', () => {
  const prices = [
    price(rate('99'), MONTHLY),
    price(rate('950'), ANNUAL),
    price(rate('89'), MONTHLY, 'EUR'),
  ]

  it('reads every cadence of the currency, monthly first', () => {
    expect(rowWords([...prices].reverse(), 'USD')).toEqual({
      amount: '$99 / month · or $950 / year',
      billed: 'billed monthly or yearly, upfront',
    })
  })

  it('has none in a currency without a price', () => {
    expect(rowWords(prices, 'GBP')).toBeUndefined()
    expect(rowWords(prices, 'EUR')?.amount).toBe('€89 / month')
  })
})

describe('chipParts', () => {
  it('sets the amount apart from what it is per', () => {
    expect(chipParts('$149 / month')).toEqual({ strong: '$149', quiet: '/ month' })
    expect(chipParts('$12 per seat / month')).toEqual({ strong: '$12', quiet: 'per seat / month' })
    expect(chipParts('first 1M free, then $0.0004 per unit')).toEqual({
      strong: 'first 1M free, then $0.0004',
      quiet: 'per unit',
    })
  })

  it('keeps several prices, or words without an amount, whole', () => {
    expect(chipParts('$99 / month · or $950 / year').quiet).toBe('')
    expect(chipParts('by rate card')).toEqual({ strong: 'by rate card', quiet: '' })
  })
})
