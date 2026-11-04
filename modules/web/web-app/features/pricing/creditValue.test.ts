import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'
import { z } from 'zod'

import { creditValue, effectiveCreditRate, packCreditPrice } from '@/features/pricing/creditValue'
import { CreditUnitPricingSchema, refineCreditUnitPricing } from '@/features/pricing/schemas'
import { CreditUnitSchema } from '@/rpc/api/creditunits/v1/models_pb'

const unit = create(CreditUnitSchema, {
  id: 'cu',
  prices: [{ id: 'p', currency: 'USD', unitPrice: '0.01' }],
})

describe('credit value', () => {
  it('is the unit price in the currency, never zero', () => {
    expect(creditValue(unit, 'usd')).toBe('0.01')
    expect(creditValue(unit, 'EUR')).toBeUndefined()
    expect(creditValue(undefined, 'USD')).toBeUndefined()
  })

  it('has no value in a currency whose price is archived', () => {
    const archived = create(CreditUnitSchema, {
      id: 'cu',
      prices: [{ id: 'p', currency: 'EUR', unitPrice: '0.01', archivedAt: '2026-10-01T00:00:00Z' }],
    })
    expect(creditValue(archived, 'EUR')).toBeUndefined()
  })

  it("falls back from the price's own rate to the unit's value", () => {
    expect(effectiveCreditRate('0.02', unit, 'USD')).toBe('0.02')
    expect(effectiveCreditRate('0', unit, 'USD')).toBe('0.01')
    expect(effectiveCreditRate('', unit, 'EUR')).toBeUndefined()
  })

  it("shows a pack's price per credit against the unit's value", () => {
    expect(packCreditPrice('90', '10000', '0.01')).toEqual({ perCredit: '0.009', discountPct: 10 })
    expect(packCreditPrice('90', '10000', undefined)).toEqual({ perCredit: '0.009' })
    expect(packCreditPrice('0', '10000', '0.01')).toBeUndefined()
  })
})

describe('overage price per credit', () => {
  const schema = CreditUnitPricingSchema.superRefine(refineCreditUnitPricing)
  const issues = (input: Partial<z.infer<typeof CreditUnitPricingSchema>>) => {
    const result = schema.safeParse({ creditUnitId: 'cu', ...input })
    return result.success ? [] : result.error.issues.map(issue => issue.message)
  }

  it('may stay empty where the unit has a value or access is denied at zero', () => {
    expect(issues({ creditUnitValued: true })).toEqual([])
    expect(issues({ creditRateOptional: true })).toEqual([])
    expect(issues({})).toEqual([
      'Enter an overage price per credit, or give the credit unit a value in this currency',
    ])
    expect(issues({ creditUnitValued: true, creditUnitRate: '0' })).toEqual([
      'The overage price must be positive, or empty',
    ])
  })
})
