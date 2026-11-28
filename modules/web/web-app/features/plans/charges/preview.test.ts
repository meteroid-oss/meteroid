import { describe, expect, it } from 'vitest'

import { allowanceRows } from './kinds'
import { invoicePreview, sampleRange, timingCaption } from './preview'

import type { ChargeShape } from './compile'

const ctx = {
  name: 'API calls',
  currency: 'USD',
  per: 'month',
  adverb: 'monthly',
  seat: 'seat',
  unit: { singular: 'call', plural: 'calls' },
  metricName: 'API calls',
}
const sample = { quantity: 2500, level: 0, later: false }

const fixed: ChargeShape = {
  source: 'fixed',
  model: 'flat',
  recurrence: 'recurring',
  billing: 'advance',
}
const seats: ChargeShape = {
  source: 'seats',
  model: 'per_unit',
  recurrence: 'recurring',
  billing: 'advance',
}
const usage: ChargeShape = {
  source: 'metered',
  model: 'per_unit',
  recurrence: 'recurring',
  billing: 'arrears',
}
const tiers: ChargeShape = { ...usage, model: 'tiered' }
const levels: ChargeShape = { ...usage, model: 'included', billing: 'advance' }

describe('invoicePreview', () => {
  it('bills a fixed fee each period, a one-time fee on the first invoice only', () => {
    const each = invoicePreview(fixed, { rate: '99' }, sample, ctx)
    expect(each?.total.toString()).toBe('99')
    expect(each?.totalLabel).toBe('Each month')
    const once: ChargeShape = { ...fixed, recurrence: 'one_time' }
    expect(invoicePreview(once, { rate: '500' }, sample, ctx)?.total.toString()).toBe('500')
    const later = invoicePreview(once, { rate: '500' }, { ...sample, later: true }, ctx)
    expect(later?.total.toString()).toBe('0')
    expect(later?.totalLabel).toBe('Next invoices')
  })

  it('counts seats', () => {
    const preview = invoicePreview(seats, { unitPrice: '12' }, { ...sample, quantity: 10 }, ctx)
    expect(preview?.total.toString()).toBe('120')
    expect(preview?.lines[0].detail).toMatch(/^10 seats × /)
  })

  it('bills usage above what is included, the included part a line of its own', () => {
    const values = { rows: allowanceRows(1000, '0.01') }
    const preview = invoicePreview(tiers, values, sample, ctx)
    expect(preview?.total.toString()).toBe('15')
    expect(preview?.lines.map(line => [line.label, line.amount.toString(), !!line.sub])).toEqual([
      ['API calls', '15', false],
      ['First 1,000 calls', '0', true],
      ['1,500 calls above it', '15', true],
    ])
  })

  it('breaks graduated tiers down, the parts summing to the line', () => {
    const rows = [
      { firstUnit: BigInt(0), unitPrice: '1' },
      { firstUnit: BigInt(1000), unitPrice: '0.5' },
    ]
    const preview = invoicePreview(tiers, { rows }, { ...sample, quantity: 1500 }, ctx)
    expect(preview?.total.toString()).toBe('1250')
    expect(preview?.lines.filter(line => line.sub).map(line => line.label)).toEqual([
      '1 – 1,000',
      '1,001 – 1,500',
    ])
  })

  it('prices the level picked, overage above it', () => {
    const thresholds = [
      { included: 10_000, rate: '199', overageRate: '0.01' },
      { included: 1000, rate: '49', overageRate: '0.02' },
    ]
    const small = invoicePreview(levels, { thresholds }, { ...sample, level: 0 }, ctx)
    expect(small?.total.toString()).toBe('79')
    const large = invoicePreview(levels, { thresholds }, { ...sample, level: 1 }, ctx)
    expect(large?.total.toString()).toBe('199')
    expect(large?.lines).toHaveLength(1)
  })

  it('has no single amount for usage priced by dimension', () => {
    const byDimension: ChargeShape = { ...usage, model: 'by_dimension' }
    expect(invoicePreview(byDimension, {}, sample, ctx)).toBeUndefined()
  })
})

describe('timingCaption', () => {
  it('says when each kind is billed, and what counts usage', () => {
    expect(timingCaption(fixed, ctx)).toBe('Billed monthly, in advance')
    expect(timingCaption({ ...fixed, recurrence: 'one_time' }, ctx)).toBe(
      'Billed once, on the first invoice'
    )
    expect(timingCaption(usage, ctx)).toBe('Billed after each month · counted by API calls')
    expect(timingCaption(levels, ctx)).toBe(
      'Fee billed in advance · usage above it after each month · counted by API calls'
    )
  })
})

describe('sampleRange', () => {
  it('starts around what the price sets', () => {
    expect(sampleRange(seats, {}).initial).toBe(10)
    expect(sampleRange(tiers, { rows: allowanceRows(1_000_000, '0.01') })).toEqual({
      initial: 2_400_000,
      max: 4_000_000,
    })
    expect(sampleRange(usage, {}).initial).toBe(1000)
    expect(sampleRange(usage, {}, { conversionFactor: 1_000_000 }).initial).toBe(10)
  })
})

describe('invoicePreview of an amount', () => {
  it('prices a percentage on the amount, and a price from events as the amount', () => {
    const percentage = invoicePreview({ ...usage, model: 'percentage' }, { percent: '2.9' }, sample, ctx)
    expect(percentage?.total.toString()).toBe('72.5')
    expect(percentage?.lines[0].detail).toBe('2.9% of $2,500.00')
    const dynamic = invoicePreview({ ...usage, model: 'dynamic' }, {}, sample, ctx)
    expect(dynamic?.total.toString()).toBe('2500')
    expect(dynamic?.lines[0].detail).toBe('$2,500.00 priced by its events')
  })
})
