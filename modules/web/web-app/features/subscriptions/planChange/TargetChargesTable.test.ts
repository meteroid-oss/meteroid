import { describe, expect, it } from 'vitest'

import { pricesKey, targetPrices } from './TargetChargesTable'

describe('targetPrices', () => {
  it('sends the prices set to a decimal, trimmed, and leaves the others at the list price', () => {
    expect(targetPrices({ platform: ' 79.00 ', seats: '', other: 'abc' })).toEqual([
      { componentId: 'platform', price: '79.00' },
    ])
  })

  it('keys the prices sent whatever their order', () => {
    expect(pricesKey({ b: '2', a: '1' })).toBe(pricesKey({ a: '1', b: '2' }))
    expect(pricesKey({ a: '1', b: 'x' })).toBe('a=1')
  })
})
