import { describe, expect, it } from 'vitest'

import { minorUnitsToInput, parseMinorUnits } from './amounts'

describe('parseMinorUnits / minorUnitsToInput at precision 2 (USD)', () => {
  it('round-trips a 2-decimal amount exactly', () => {
    expect(parseMinorUnits('40.50', 'USD')).toBe(4050n)
    expect(minorUnitsToInput(4050n, 'USD')).toBe('40.5')
  })

  it('rejects an amount with more decimals than the currency supports', () => {
    expect(parseMinorUnits('40.505', 'USD')).toBeNull()
  })

  it('accepts a whole amount', () => {
    expect(parseMinorUnits('40', 'USD')).toBe(4000n)
  })
})

describe('parseMinorUnits / minorUnitsToInput at precision 0 (JPY)', () => {
  it('treats the major unit as the minor unit — no /100 scaling', () => {
    expect(parseMinorUnits('15000', 'JPY')).toBe(15000n)
    expect(minorUnitsToInput(15000n, 'JPY')).toBe('15000')
  })

  it('rejects any fractional amount', () => {
    expect(parseMinorUnits('15000.5', 'JPY')).toBeNull()
  })
})

describe('parseMinorUnits edge cases', () => {
  it('rejects empty, negative and non-numeric input', () => {
    expect(parseMinorUnits('', 'USD')).toBeNull()
    expect(parseMinorUnits('-5', 'USD')).toBeNull()
    expect(parseMinorUnits('abc', 'USD')).toBeNull()
  })
})
