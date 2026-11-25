import { describe, expect, it } from 'vitest'

import { parseRateLines } from './rateLines'

describe('rate lines', () => {
  it('reads a pattern per dimension, a currency and a rate', () => {
    expect(parseRateLines('anthropic/*, input, usd, 3.00\n', 2, true)).toEqual({
      lines: [{ key: ['anthropic/*', 'input'], currency: 'USD', rate: '3.00' }],
    })
  })

  it('reads a credit card without currency, tabs and a header line', () => {
    expect(parseRateLines('model\trate\ngpt-4o\t2.5', 1, false)).toEqual({
      lines: [{ key: ['gpt-4o'], rate: '2.5' }],
    })
  })

  it('names the line it cannot read', () => {
    expect(parseRateLines('x,USD,0.1\ny,USD,abc', 1, true)).toEqual({
      error: 'Line 2: "abc" is not a rate',
    })
    expect(parseRateLines('x,0.1', 1, true)).toEqual({
      error: 'Line 1: 1 patterns, a currency and a rate expected',
    })
    expect(parseRateLines('x,dollars,0.1', 1, true)).toEqual({
      error: 'Line 1: "dollars" is not a currency code',
    })
  })
})
