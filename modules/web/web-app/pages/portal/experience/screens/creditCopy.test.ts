import { describe, expect, it } from 'vitest'

import { creditPaysLine, roundCreditPresets, topUpSettlesCopy } from './creditCopy'

const unit = { name: 'API credit', pluralName: 'API credits' }

describe('creditPaysLine', () => {
  it('says credits pay usage and money pays invoices', () => {
    expect(creditPaysLine([{ creditUnit: undefined }])).toBe(
      'Applied to your invoices automatically.'
    )
    expect(creditPaysLine([{ creditUnit: unit as never }])).toBe(
      'Applied to your usage automatically.'
    )
    expect(creditPaysLine([{ creditUnit: unit as never }, { creditUnit: undefined }])).toBe(
      'Applied to your usage and invoices automatically.'
    )
  })
})

describe('roundCreditPresets', () => {
  it('rounds to 1-2-2.5-5 counts of whole credits', () => {
    expect(roundCreditPresets('0.0008', 0, 'EUR')).toEqual([25000n, 50000n, 100000n, 250000n])
    expect(roundCreditPresets('0.003', 0, 'USD')).toEqual([10000n, 20000n, 25000n, 100000n])
  })

  it('scales whole credits to the unit precision', () => {
    expect(roundCreditPresets('0.001', 2, 'USD')).toEqual([
      2500000n,
      5000000n,
      10000000n,
      25000000n,
    ])
  })

  it('keeps distinct presets of at least one credit', () => {
    expect(roundCreditPresets('100', 0, 'USD')).toEqual([1n, 2n])
  })

  it('uses zero-decimal money presets', () => {
    expect(roundCreditPresets('0.5', 0, 'JPY')).toEqual([2000n, 5000n, 10000n, 20000n])
  })

  it('offers nothing for an invalid price', () => {
    expect(roundCreditPresets('0', 0, 'USD')).toEqual([])
  })
})

describe('topUpSettlesCopy', () => {
  it('says credits pay usage and money pays invoices', () => {
    expect(topUpSettlesCopy(unit)).toBe(
      'the API credits are added once the payment settles. They pay your usage.'
    )
    expect(topUpSettlesCopy()).toBe(
      'the credit is added once the payment settles. It pays your next invoices.'
    )
  })
})
