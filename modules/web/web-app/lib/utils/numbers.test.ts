import { describe, expect, it } from 'vitest'

import { formatCurrency, formatUsage } from './numbers'

describe('formatUsage grouping', () => {
  it('groups thousands and caps small (<100) quantities at 4 decimals', () => {
    expect(formatUsage(1234.5)).toBe('1,234.5')
    expect(formatUsage(12.34567)).toBe('12.3457')
  })

  it('caps large (>=100) quantities at 2 decimals', () => {
    expect(formatUsage(123456.789)).toBe('123,456.79')
  })

  it('caps sub-1 quantities at 12 decimals, trimming trailing zeros', () => {
    expect(formatUsage(0.000123456789)).toBe('0.000123456789')
    expect(formatUsage(0.5)).toBe('0.5')
  })
})

describe('formatCurrency precision', () => {
  it('keeps 2-decimal currencies unchanged', () => {
    expect(formatCurrency(12345, 'USD')).toBe('$123.45')
  })

  it('formats a 0-decimal currency (JPY) without dividing by 100', () => {
    expect(formatCurrency(12345, 'JPY')).toBe('¥12,345')
  })

  it('formats a 3-decimal currency (KWD)', () => {
    expect(formatCurrency(12345, 'KWD')).toBe('KWD\u00A012.345')
  })
})
