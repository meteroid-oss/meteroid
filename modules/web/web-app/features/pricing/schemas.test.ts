import { describe, expect, it } from 'vitest'
import { z } from 'zod'

import { CreditPricingSchema, refineCreditPricing } from '@/features/pricing/schemas'

import type { CreditPackContext } from '@/features/pricing/schemas'

const units: Record<string, number> = { requests: 0 }
const schema = (currency?: string) => {
  const context: CreditPackContext | undefined = currency
    ? { currency, unitPrecision: id => units[id] }
    : undefined
  return CreditPricingSchema.superRefine((data, ctx) => refineCreditPricing(data, ctx, context))
}
const pack = (fields: Record<string, unknown>) => ({
  unitPrice: '10',
  quantity: 1,
  creditAmount: '10',
  ...fields,
})
const messages = (s: z.ZodTypeAny, input: unknown) => {
  const result = s.safeParse(input)
  return result.success ? [] : result.error.issues.map(issue => `${issue.path.join('.')}: ${issue.message}`)
}

describe('credit pack refinements', () => {
  it('refuses a money pack dearer than its credit, not a unit pack', () => {
    expect(messages(schema(), pack({ unitPrice: '12' }))).toEqual([
      'creditAmount: A pack cannot cost more than the credit it grants',
    ])
    expect(messages(schema(), pack({ unitPrice: '10' }))).toEqual([])
    expect(messages(schema(), pack({ unitPrice: '12', grantCreditUnitId: 'requests' }))).toEqual([])
  })

  it('counts the credit in the currency, or the unit, precision', () => {
    expect(messages(schema('JPY'), pack({ unitPrice: '0', creditAmount: '10.5' }))).toEqual([
      'creditAmount: Enter a whole number',
    ])
    expect(messages(schema('KWD'), pack({ unitPrice: '0', creditAmount: '10.125' }))).toEqual([])
    expect(messages(schema('EUR'), pack({ unitPrice: '0', creditAmount: '10.125' }))).toEqual([
      'creditAmount: Enter at most 2 decimals',
    ])
    expect(
      messages(schema('EUR'), pack({ creditAmount: '10.5', grantCreditUnitId: 'requests' }))
    ).toEqual(['creditAmount: Enter a whole number'])
  })

  it('checks the rollover cap precision and term like the server', () => {
    const rollover = (fields: Record<string, unknown>) =>
      pack({
        unitPrice: '0',
        expiryUnit: 'BILLING_PERIOD',
        rolloverEnabled: true,
        ...fields,
      })
    expect(messages(schema('JPY'), rollover({ rolloverMaxAmount: '5.5' }))).toEqual([
      'rolloverMaxAmount: Enter a whole number',
    ])
    expect(
      messages(schema('EUR'), rollover({ rolloverExpiryUnit: 'MONTH', rolloverExpiryValue: 121 }))
    ).toEqual(['rolloverExpiryValue: Enter at most 120 months'])
    expect(
      messages(schema('EUR'), rollover({ rolloverExpiryUnit: 'DAY', rolloverExpiryValue: 3660 }))
    ).toEqual([])
    expect(
      messages(schema('EUR'), pack({ rolloverMaxAmount: '5.555', rolloverEnabled: false }))
    ).toEqual([])
  })

  // The switch stays on while its fields are hidden: a paid, never-expiring or once-granted pack
  // sends no rollover, so its stale cap and term must not block the save.
  it('ignores a rollover left switched on where the pack cannot roll over', () => {
    const stale = { rolloverEnabled: true, rolloverMaxAmount: '5.55', grantCreditUnitId: 'requests' }
    const term = { rolloverExpiryUnit: 'MONTH', rolloverExpiryValue: 200 }
    for (const fields of [
      { unitPrice: '10', expiryUnit: 'BILLING_PERIOD' },
      { unitPrice: '0', expiryUnit: 'NONE' },
    ]) {
      expect(messages(schema('EUR'), pack({ ...stale, ...term, ...fields }))).toEqual([])
    }
    expect(
      messages(schema('EUR'), pack({ ...stale, unitPrice: '0', expiryUnit: 'BILLING_PERIOD' }))
    ).toEqual(['rolloverMaxAmount: Enter a whole number'])
  })
})
